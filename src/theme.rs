use eframe::egui::style::WidgetVisuals;
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, TextureOptions,
};

pub const CANVAS: Color32 = Color32::from_rgb(11, 18, 32);
pub const SURFACE: Color32 = Color32::from_rgb(17, 28, 45);
pub const RAISED: Color32 = Color32::from_rgb(24, 38, 58);
pub const BORDER: Color32 = Color32::from_rgb(40, 56, 77);
pub const TEXT: Color32 = Color32::from_rgb(240, 245, 252);
pub const MUTED: Color32 = Color32::from_rgb(163, 179, 201);
pub const CYAN: Color32 = Color32::from_rgb(19, 215, 241);
pub const BLUE: Color32 = Color32::from_rgb(8, 123, 255);
pub const GOLD: Color32 = Color32::from_rgb(255, 200, 71);
pub const ERROR: Color32 = Color32::from_rgb(240, 107, 117);
pub const SUCCESS: Color32 = Color32::from_rgb(69, 201, 138);

pub fn apply(ctx: &egui::Context) {
    let mut style = egui::Style {
        text_styles: [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(20.0, FontFamily::Proportional),
            ),
        ]
        .into(),
        ..Default::default()
    };
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.window_margin = egui::Margin::same(18);
    style.spacing.menu_margin = egui::Margin::same(10);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size = egui::vec2(42.0, 38.0);
    style.spacing.indent = 18.0;
    style.spacing.icon_width = 18.0;
    style.spacing.icon_width_inner = 11.0;
    style.spacing.icon_spacing = 8.0;
    style.spacing.extra_text_line_spacing = 2.0;
    style.animation_time = 0.12;

    let radius = CornerRadius::same(6);
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.weak_text_color = Some(MUTED);
    visuals.hyperlink_color = CYAN;
    visuals.faint_bg_color = SURFACE;
    visuals.extreme_bg_color = CANVAS;
    visuals.text_edit_bg_color = Some(SURFACE);
    visuals.code_bg_color = CANVAS;
    visuals.error_fg_color = ERROR;
    visuals.window_corner_radius = CornerRadius::same(14);
    visuals.window_fill = SURFACE;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.menu_corner_radius = radius;
    visuals.panel_fill = CANVAS;
    visuals.selection.bg_fill = BLUE;
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    visuals.button_frame = true;
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = false;
    visuals.striped = true;
    visuals.slider_trailing_fill = true;
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.widgets.noninteractive = widget(SURFACE, SURFACE, BORDER, MUTED, radius, 0.0);
    visuals.widgets.inactive = widget(RAISED, RAISED, BORDER, TEXT, radius, 0.0);
    visuals.widgets.hovered = widget(
        Color32::from_rgb(28, 51, 76),
        Color32::from_rgb(28, 51, 76),
        CYAN,
        TEXT,
        radius,
        1.0,
    );
    visuals.widgets.active = widget(BLUE, BLUE, CYAN, TEXT, radius, 0.0);
    visuals.widgets.open = widget(RAISED, RAISED, CYAN, TEXT, radius, 0.0);
    style.visuals = visuals;

    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub fn load_brand(ctx: &egui::Context) -> egui::TextureHandle {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/aitierlist.png"))
        .expect("embedded brand icon must be a valid PNG");
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    );
    ctx.load_texture("aitierlist-brand", image, TextureOptions::LINEAR)
}

fn widget(
    bg_fill: Color32,
    weak_bg_fill: Color32,
    border: Color32,
    foreground: Color32,
    corner_radius: CornerRadius,
    expansion: f32,
) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill,
        weak_bg_fill,
        bg_stroke: Stroke::new(1.0, border),
        corner_radius,
        fg_stroke: Stroke::new(1.0, foreground),
        expansion,
    }
}
