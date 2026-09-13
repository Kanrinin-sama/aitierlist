#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopEvent {
    Open,
    Subscriptions,
    Quit,
}

#[cfg(windows)]
mod platform {
    use super::DesktopEvent;
    use anyhow::{Context, Result, bail};
    use eframe::egui;
    use std::mem::size_of;
    use std::ptr::{null, null_mut};
    use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError,
        HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DestroyWindow, DispatchMessageW, FindWindowW, GWLP_USERDATA, GetMessageW, HWND_BROADCAST,
        IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED, LoadImageW, MF_STRING, MSG, PostMessageW,
        PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_RESTORE, SetForegroundWindow,
        SetWindowLongPtrW, ShowWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
        TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY,
        WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    };

    const CALLBACK_MESSAGE: u32 = WM_APP + 1;
    const OPEN_ID: usize = 1;
    const SUBSCRIPTIONS_ID: usize = 2;
    const QUIT_ID: usize = 3;
    const ICON_ID: u32 = 1;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn class_name() -> Vec<u16> {
        wide("AiTierListDesktopWindow")
    }

    fn instance_mutex_name() -> Vec<u16> {
        wide("Local\\AiTierListDesktopInstance")
    }

    fn instance_event_name() -> Vec<u16> {
        wide("Local\\AiTierListDesktopOpen")
    }

    pub struct InstanceGuard {
        mutex: HANDLE,
        event: HANDLE,
    }

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.event);
                CloseHandle(self.mutex);
            }
        }
    }

    pub fn claim_single_instance() -> Result<Option<InstanceGuard>> {
        unsafe {
            let mutex = CreateMutexW(null(), 0, instance_mutex_name().as_ptr());
            if mutex.is_null() {
                bail!(
                    "creating the application instance mutex failed: {}",
                    GetLastError()
                );
            }
            if GetLastError() != ERROR_ALREADY_EXISTS {
                let event = CreateEventW(null(), 1, 0, instance_event_name().as_ptr());
                if event.is_null() {
                    CloseHandle(mutex);
                    bail!(
                        "creating the application instance event failed: {}",
                        GetLastError()
                    );
                }
                return Ok(Some(InstanceGuard { mutex, event }));
            }
            CloseHandle(mutex);
            let event = CreateEventW(null(), 1, 0, instance_event_name().as_ptr());
            if event.is_null() {
                bail!(
                    "opening the application instance event failed: {}",
                    GetLastError()
                );
            }
            SetEvent(event);
            CloseHandle(event);
            let message = RegisterWindowMessageW(wide("AiTierListDesktopOpenMessage").as_ptr());
            if message != 0 {
                PostMessageW(HWND_BROADCAST, message, 0, 0);
            }
            let window = FindWindowW(null(), wide("AI Tier List").as_ptr());
            if !window.is_null() {
                ShowWindow(window, SW_RESTORE);
                SetForegroundWindow(window);
            }
            Ok(None)
        }
    }

    struct WindowState {
        events: Sender<DesktopEvent>,
        context: Arc<Mutex<Option<egui::Context>>>,
        taskbar_created: u32,
        open_message: u32,
    }

    fn notify(hwnd: HWND, operation: u32) -> bool {
        unsafe {
            let module = GetModuleHandleW(null());
            let icon = LoadImageW(
                module,
                ICON_ID as usize as *const u16,
                IMAGE_ICON,
                0,
                0,
                LR_DEFAULTSIZE | LR_SHARED,
            );
            if icon.is_null() {
                return false;
            }
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = hwnd;
            data.uID = ICON_ID;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = CALLBACK_MESSAGE;
            data.hIcon = icon;
            let tooltip = wide("AI Tier List");
            data.szTip[..tooltip.len()].copy_from_slice(&tooltip);
            Shell_NotifyIconW(operation, &data) != 0
        }
    }

    fn emit(state: &WindowState, event: DesktopEvent) {
        let _ = state.events.send(event);
        let context = state
            .context
            .lock()
            .ok()
            .and_then(|context| context.clone());
        if let Some(context) = context {
            match event {
                DesktopEvent::Open | DesktopEvent::Subscriptions => {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                DesktopEvent::Quit => context.send_viewport_cmd(egui::ViewportCommand::Close),
            }
            context.request_repaint();
        }
        let application_window = unsafe { FindWindowW(null(), wide("AI Tier List").as_ptr()) };
        if !application_window.is_null() {
            unsafe {
                match event {
                    DesktopEvent::Open | DesktopEvent::Subscriptions => {
                        ShowWindow(application_window, SW_RESTORE);
                        SetForegroundWindow(application_window);
                    }
                    DesktopEvent::Quit => {
                        PostMessageW(application_window, WM_CLOSE, 0, 0);
                    }
                }
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_CREATE {
            let create = lparam as *const CREATESTRUCTW;
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize) };
            return 0;
        }
        let state = unsafe {
            (windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
                as *const WindowState)
                .as_ref()
        };
        if let Some(state) = state {
            if message == state.taskbar_created {
                if !notify(hwnd, NIM_ADD) {
                    emit(state, DesktopEvent::Open);
                }
                return 0;
            }
            if message == state.open_message {
                emit(state, DesktopEvent::Open);
                return 0;
            }
            if message == CALLBACK_MESSAGE {
                if lparam as u32 == WM_LBUTTONDBLCLK {
                    emit(state, DesktopEvent::Open);
                } else if lparam as u32 == WM_RBUTTONUP {
                    show_menu(hwnd);
                }
                return 0;
            }
            if message == WM_COMMAND {
                match wparam & 0xffff {
                    OPEN_ID => emit(state, DesktopEvent::Open),
                    SUBSCRIPTIONS_ID => emit(state, DesktopEvent::Subscriptions),
                    QUIT_ID => emit(state, DesktopEvent::Quit),
                    _ => {}
                }
                return 0;
            }
        }
        match message {
            WM_CLOSE => unsafe {
                DestroyWindow(hwnd);
            },
            WM_DESTROY => {
                let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
                data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                data.hWnd = hwnd;
                data.uID = ICON_ID;
                unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
                unsafe { PostQuitMessage(0) };
            }
            _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
        0
    }

    fn show_menu(hwnd: HWND) {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(menu, MF_STRING, OPEN_ID, wide("Open").as_ptr());
            AppendMenuW(
                menu,
                MF_STRING,
                SUBSCRIPTIONS_ID,
                wide("Subscriptions").as_ptr(),
            );
            AppendMenuW(menu, MF_STRING, QUIT_ID, wide("Quit").as_ptr());
            let mut point = std::mem::zeroed();
            if windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) != 0 {
                SetForegroundWindow(hwnd);
                TrackPopupMenu(
                    menu,
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    0,
                    hwnd,
                    null(),
                );
                PostMessageW(hwnd, WM_NULL, 0, 0);
            }
            DestroyMenu(menu);
        }
    }

    pub struct Desktop {
        events: Receiver<DesktopEvent>,
        context: Arc<Mutex<Option<egui::Context>>>,
        hwnd: isize,
        thread: Option<JoinHandle<()>>,
    }

    impl Desktop {
        pub fn new(context: Option<egui::Context>) -> Result<Self> {
            let (events_tx, events) = channel();
            let (ready_tx, ready_rx) = sync_channel(1);
            let context = Arc::new(Mutex::new(context));
            let thread_context = Arc::clone(&context);
            let thread = std::thread::spawn(move || run(events_tx, thread_context, ready_tx));
            let hwnd = ready_rx
                .recv()
                .context("the desktop integration thread stopped")?
                .map_err(anyhow::Error::msg)?;
            Ok(Self {
                events,
                context,
                hwnd,
                thread: Some(thread),
            })
        }

        pub fn next_event(&self) -> Option<DesktopEvent> {
            self.events.try_recv().ok()
        }

        pub fn wait_event(&self) -> Option<DesktopEvent> {
            self.events.recv().ok()
        }

        pub fn attach_context(&self, context: egui::Context) {
            if let Ok(mut current) = self.context.lock() {
                *current = Some(context);
            }
        }
    }

    impl Drop for Desktop {
        fn drop(&mut self) {
            unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn run(
        events: Sender<DesktopEvent>,
        context: Arc<Mutex<Option<egui::Context>>>,
        ready: std::sync::mpsc::SyncSender<Result<isize, String>>,
    ) {
        unsafe {
            let module = GetModuleHandleW(null());
            let class = class_name();
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: module,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&window_class) == 0 {
                let _ = ready.send(Err(format!(
                    "registering the desktop window failed: {}",
                    GetLastError()
                )));
                return;
            }
            let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            let open_message =
                RegisterWindowMessageW(wide("AiTierListDesktopOpenMessage").as_ptr());
            let state = Box::new(WindowState {
                events,
                context,
                taskbar_created,
                open_message,
            });
            let state_ptr = Box::into_raw(state);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("AI Tier List Tray Host").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                module,
                state_ptr.cast(),
            );
            if hwnd.is_null() {
                drop(Box::from_raw(state_ptr));
                let _ = ready.send(Err(format!(
                    "creating the desktop window failed: {}",
                    GetLastError()
                )));
                return;
            }
            if !notify(hwnd, NIM_ADD) {
                DestroyWindow(hwnd);
                drop(Box::from_raw(state_ptr));
                let _ = ready.send(Err(format!(
                    "creating the notification icon failed: {}",
                    GetLastError()
                )));
                return;
            }
            let open_event = CreateEventW(null(), 1, 0, instance_event_name().as_ptr());
            if !open_event.is_null() && WaitForSingleObject(open_event, 0) == WAIT_OBJECT_0 {
                emit(&*state_ptr, DesktopEvent::Open);
            }
            if !open_event.is_null() {
                CloseHandle(open_event);
            }
            let _ = ready.send(Ok(hwnd as isize));
            let mut message: MSG = std::mem::zeroed();
            loop {
                let result = GetMessageW(&mut message, null_mut(), 0, 0);
                if result > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                } else {
                    if result == -1 {
                        DestroyWindow(hwnd);
                    }
                    break;
                }
            }
            drop(Box::from_raw(state_ptr));
        }
    }

    pub fn set_start_at_login(enabled: bool, start_minimized: bool) -> Result<()> {
        let command = if enabled {
            let executable =
                std::env::current_exe().context("locating the application executable")?;
            let suffix = if start_minimized {
                " --start-minimized"
            } else {
                ""
            };
            Some(wide(&format!("\"{}\"{suffix}", executable.display())))
        } else {
            None
        };
        unsafe {
            let key_path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            let mut key = null_mut();
            let status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key_path.as_ptr(),
                0,
                null_mut(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                null(),
                &mut key,
                null_mut(),
            );
            if status != ERROR_SUCCESS {
                bail!("opening the startup registry key failed: {status}");
            }
            let name = wide("AiTierList");
            let status = if let Some(command) = command {
                RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * size_of::<u16>()) as u32,
                )
            } else {
                let result = RegDeleteValueW(key, name.as_ptr());
                if result == ERROR_FILE_NOT_FOUND {
                    ERROR_SUCCESS
                } else {
                    result
                }
            };
            RegCloseKey(key);
            if status != ERROR_SUCCESS {
                bail!("updating startup registration failed: {status}");
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
pub use platform::{Desktop, claim_single_instance, set_start_at_login};

#[cfg(not(windows))]
pub struct Desktop;

#[cfg(not(windows))]
impl Desktop {
    pub fn new(_: Option<eframe::egui::Context>) -> anyhow::Result<Self> {
        anyhow::bail!("desktop integration is available on Windows")
    }
    pub fn next_event(&self) -> Option<DesktopEvent> {
        None
    }
    pub fn wait_event(&self) -> Option<DesktopEvent> {
        None
    }
    pub fn attach_context(&self, _: eframe::egui::Context) {}
}

#[cfg(not(windows))]
pub struct InstanceGuard;

#[cfg(not(windows))]
pub fn claim_single_instance() -> anyhow::Result<Option<InstanceGuard>> {
    Ok(Some(InstanceGuard))
}

#[cfg(not(windows))]
pub fn set_start_at_login(_: bool, _: bool) -> anyhow::Result<()> {
    anyhow::bail!("startup registration is available on Windows")
}
