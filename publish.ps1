#requires -Version 7
<#
.SYNOPSIS
    Publish an AITIERLIST release to files.blockitall.us (served by strimr-server on kanrinin).

.DESCRIPTION
    Mirrors the ShareX release layout already on that host:

        /aitierlist/update.json
        /aitierlist/releases/<version>/aitierlist-<version>-portable-x64.exe

    Ordering matters and is deliberate: the artifact is installed and re-verified
    *on the server*, and only then is update.json promoted. Writing the manifest
    first would leave a window where a client could fetch a manifest pointing at
    a half-transferred file.

    The installation itself is a transaction, not a sequence of moves. Both
    objects are staged in the publisher's home directory, verified there, copied
    into hidden temporaries that already live in their own final directories,
    and verified again - and only then, under one server-side lock and after a
    compare-and-set recheck of the manifest that is currently published, are
    they renamed into place. Every rename is inside one directory, so it is
    atomic and no client can ever see a partially copied release. A version that
    is already published with these exact bytes is left completely alone.

    Every native command is bounded and noninteractive. PowerShell does not raise
    on a non-zero exit from ssh or curl - $ErrorActionPreference has no effect on
    them - so this script used to sail past a failed upload, a failed remote move
    and a failed manifest write and print "published." at the end regardless.
    Every child now goes through Invoke-BoundedProcess, which has one wall-clock
    deadline over the whole exchange, independent output bounds on both pipes,
    and no path to a password prompt.

.PARAMETER Version
    Defaults to the version in Cargo.toml.

.PARAMETER Force
    Repair a publish of THESE EXACT BYTES that did not finish - a manifest that
    was never written, a live check that could not run. It is not permission to
    replace anything: a published version is immutable, an artifact with
    different bytes is refused whatever this flag says, and an artifact with
    identical bytes is left exactly as it is (not renamed over, not chmod-ed).
    All -Force permits is rewriting the manifest idempotently, and only when
    the candidate is not older than the version the live channel currently
    names. An older checkout cannot repoint update.json at a superseded
    release.

.PARAMETER WhatIf
    Show what would happen, touch nothing.
#>

[CmdletBinding()]
param(
    [string] $Version,
    [string] $RemoteHost = 'kanrinin',
    [string] $WebRoot = '/opt/homebrew/var/www/files/aitierlist',
    [switch] $Force,
    [switch] $WhatIf,
    [switch] $AllowUnbound
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = $PSScriptRoot
$exe = Join-Path $root 'target\release\aitierlist.exe'
$packedExe = Join-Path $root 'target\release\aitierlist-packed.exe'
$upx = 'C:\Users\kaltsit\AppData\Local\Microsoft\WinGet\Links\upx.exe'
if (-not (Test-Path -LiteralPath $upx -PathType Leaf)) { throw "UPX was not found at $upx" }

# ---------------------------------------------------------------- primitives

# The pure decisions live beside this script, apart from anything that uploads.
#requires -Version 7
<#
.SYNOPSIS
    The decisions publish.ps1 makes that are pure functions of their input.

.DESCRIPTION
    Split out so they can be tested without uploading anything. Almost
    everything here takes values and returns values: no network, no remote host,
    no filesystem beyond the System32 lookup, and nothing that depends on having
    a release build to hand. The two exceptions are deliberate and are still
    testable against a local stand-in child: the bounded process runner and the
    object-bound remote write built on it. publish.ps1 dot-sources this file and
    does the rest.
#>

Set-StrictMode -Version Latest

# Resolve a Windows tool by absolute path under System32, never by bare name.
#
# `ssh`, `scp` and `curl.exe` off PATH are whatever the caller's environment
# points at, and the publish script hands them a remote host, a web root and the
# bytes of a release. The OpenSSH client and curl both ship in System32 on every
# supported Windows; nothing here falls back to a search.
function Resolve-SystemTool {
    param([Parameter(Mandatory)][string] $Leaf)

    $system32 = [System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::System)
    if ([string]::IsNullOrWhiteSpace($system32)) { throw 'could not resolve the Windows system directory' }
    $candidates = @(
        (Join-Path $system32 $Leaf),
        (Join-Path (Join-Path $system32 'OpenSSH') $Leaf)
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    throw "$Leaf was not found under $system32 (looked in System32 and System32\OpenSSH)"
}

# A remote host is a name the script quotes into a command line and hands to
# ssh; a web root is a path it interpolates into a remote shell command. Both are
# parameters with defaults, so both are also where an operator's typo - or a
# pasted string - becomes remote command injection. Restricting them to the
# characters a host name and an absolute POSIX path may contain removes the
# quoting question entirely rather than answering it.
function Assert-Grammar {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Value,
        [Parameter(Mandatory)][string] $Pattern,
        [Parameter(Mandatory)][string] $What
    )
    if ($Value -cnotmatch $Pattern) { throw "$What '$Value' is not acceptable" }
    return $Value
}

# The absolute remote staging directory this run created, as the remote shell
# resolved it.
#
# The path is interpolated into single-quoted remote commands, so it has to be
# absolute - a tilde is not expanded inside single quotes - and it must not
# carry a quote, a backslash or anything that can end a shell word. It must also
# end in this run's own leaf, so a remote that answered with some other
# directory cannot redirect the upload into it.
function Assert-RemoteStagePath {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Path,
        [Parameter(Mandatory)][string] $Leaf
    )

    if ($Path -cnotmatch '^/[^\r\n''"\\`$]*$') {
        throw "the remote staging directory '$Path' is not a usable absolute path"
    }
    if ($Path -cmatch '(^|/)\.\.(/|$)') {
        throw "the remote staging directory '$Path' walks upwards"
    }
    $expected = "/$Leaf"
    if (-not $Path.EndsWith($expected, [System.StringComparison]::Ordinal)) {
        throw "the remote staging directory '$Path' is not this run's own '$Leaf'"
    }
    return $Path
}

# Version text the publisher and the client both accept: exactly three numeric
# components, an optional prerelease suffix, and at most 32 characters - the
# same ceiling StampScan::window() uses, so a value one gate can state is a
# value the other can scan. Two or four numeric components are not a version
# on either side of the boundary; a prerelease is a different value from its
# stable prefix, not a truncation of it.
$script:VersionBody = '[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?'
$script:VersionGrammar = '^(?=.{1,32}$)' + $script:VersionBody + '$'
$script:PublishedVersionGrammar = '^(absent|(?=.{1,32}$)' + $script:VersionBody + ')$'
$script:HostGrammar = '^[A-Za-z0-9][A-Za-z0-9._-]{0,253}$'
$script:WebRootGrammar = '^(/[A-Za-z0-9._-]+)+$'
$script:Sha256Grammar = '^[0-9a-f]{64}$'
$script:StageIdGrammar = '^[0-9a-f]{32}$'
$script:GitObjectIdGrammar = '^[0-9a-f]{40}$'
$script:ArtifactNameGrammar = '^aitierlist-' + $script:VersionBody + '-portable-x64\.exe$'
$script:ManifestCasGrammar = '^(absent|[0-9a-f]{64} [0-9]{1,19})$'
$script:VersionStampPattern = 'AITIERLIST_VERSION=([0-9][0-9.]{0,31}(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?)'
$script:CommitStampPattern = 'AITIERLIST_COMMIT=([0-9a-f]{40})(?![0-9a-f])'
$script:TreeStampPattern = 'AITIERLIST_TREE=([0-9a-f]{40})(?![0-9a-f])'
$script:MaxArtifactBytes = [long](512 * 1024 * 1024)
$script:PublishLockStaleMinutes = 15

function Get-HostGrammar { return $script:HostGrammar }
function Get-WebRootGrammar { return $script:WebRootGrammar }
function Get-VersionGrammar { return $script:VersionGrammar }
function Get-Sha256Grammar { return $script:Sha256Grammar }
function Get-StageIdGrammar { return $script:StageIdGrammar }
function Get-GitObjectIdGrammar { return $script:GitObjectIdGrammar }
function Get-ArtifactNameGrammar { return $script:ArtifactNameGrammar }
function Get-ManifestCasGrammar { return $script:ManifestCasGrammar }
function Get-VersionStampPattern { return $script:VersionStampPattern }
function Get-MaxArtifactBytes { return $script:MaxArtifactBytes }
function Get-PublishLockStaleMinutes { return $script:PublishLockStaleMinutes }

# Every complete version stamp an image carries, deduplicated.
#
# `Contains` accepted a superset and that is the whole bug: a binary built for
# 0.5.90 contains the substring "AITIERLIST_VERSION=0.5.9", and so does one built for
# 0.5.9.1, so both published happily as 0.5.9 - a release that reports a
# different version once installed, which makes every client re-offer the same
# update forever. Enumerating and comparing whole values cannot do that.
#
# The numeric run is `[0-9][0-9.]{0,31}` so a stamp cannot outgrow the 32-byte
# value StampScan will actually keep. The optional `-[A-Za-z0-9.]+` suffix is
# why `0.8.4-rc.1` is not the stamp `0.8.4`: stopping at the hyphen published
# a prerelease binary as stable.
function Get-LabeledStamps {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Image,
        [Parameter(Mandatory)][string] $Pattern,
        [int] $MaximumLength = 0
    )

    $matched = [regex]::Matches($Image, $Pattern)
    return @($matched | ForEach-Object {
            $value = $_.Groups[1].Value
            if ($MaximumLength -le 0 -or $value.Length -le $MaximumLength) { $value }
        } | Sort-Object -Unique)
}

function Get-VersionStamps {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Image)

    return @(Get-LabeledStamps -Image $Image -Pattern $script:VersionStampPattern -MaximumLength 32)
}

function Get-CommitStamps {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Image)

    return @(Get-LabeledStamps -Image $Image -Pattern $script:CommitStampPattern)
}

function Get-TreeStamps {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Image)

    return @(Get-LabeledStamps -Image $Image -Pattern $script:TreeStampPattern)
}

# The stamp check: exactly one distinct stamp, and it equals this version.
function Assert-VersionStamp {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Image,
        [Parameter(Mandatory)][string] $Version,
        [string] $Path = 'the release binary'
    )

    # @() at the caller, deliberately. PowerShell unwraps a one-element pipeline
    # result to a scalar and an empty one to $null, so `$found.Count` was
    # reading a property off a string, or off nothing at all - which under
    # `Set-StrictMode -Version Latest` is a terminating error inside the very
    # check that is meant to produce a clear message.
    $found = @(Get-VersionStamps -Image $Image)
    if ($found.Count -ne 1) {
        $what = if ($found.Count -eq 0) { 'no version stamp found' } else { "found $($found -join ', ')" }
        throw "$Path does not carry exactly one version stamp ($what). Run: cargo build --release"
    }
    if ($found[0] -cne $Version) {
        throw "$Path was not built from version $Version (it is $($found[0])). Run: cargo build --release"
    }
    return $found[0]
}

function Assert-GitObjectId {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Value,
        [Parameter(Mandatory)][string] $What
    )

    if ($Value -cnotmatch $script:GitObjectIdGrammar) {
        throw "$What '$Value' is not a 40-digit lowercase git object id"
    }
    return $Value
}

# The artifact is this HEAD, or it is someone else's binary. A version stamp
# alone cannot tell them apart: two builds of 0.8.3, nine hours and several
# commits apart, both carry AITIERLIST_VERSION=0.8.3. The commit and tree stamps
# are the bind; anything else is a stale same-version image.
function Assert-HeadBinding {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Image,
        [Parameter(Mandatory)][string] $Commit,
        [Parameter(Mandatory)][string] $Tree,
        [string] $Path = 'the release binary',
        [switch] $AllowUnbound
    )

    Assert-GitObjectId -Value $Commit -What 'HEAD commit' | Out-Null
    Assert-GitObjectId -Value $Tree -What 'HEAD tree' | Out-Null
    $commits = @(Get-CommitStamps -Image $Image)
    $trees = @(Get-TreeStamps -Image $Image)
    if ($commits.Count -ne 1) {
        $what = if ($commits.Count -eq 0) { 'no commit stamp found' } else { "found $($commits -join ', ')" }
        throw "$Path does not carry exactly one commit stamp ($what). Rebuild from a tree that embeds AITIERLIST_COMMIT."
    }
    if ($trees.Count -ne 1) {
        $what = if ($trees.Count -eq 0) { 'no tree stamp found' } else { "found $($trees -join ', ')" }
        throw "$Path does not carry exactly one tree stamp ($what). Rebuild from a tree that embeds AITIERLIST_TREE."
    }
    $allZeros = '0' * 40
    if ($commits[0] -ceq $allZeros -and $trees[0] -ceq $allZeros) {
        if (-not $AllowUnbound) {
            throw "$Path carries unbound (all-zero) commit and tree stamps. Pass -AllowUnbound to allow publishing unbound binaries."
        }
    } else {
        if ($commits[0] -cne $Commit) {
            throw "$Path was not built from HEAD $Commit (it is $($commits[0])). Rebuild from the current tree."
        }
        if ($trees[0] -cne $Tree) {
            throw "$Path was not built from tree $Tree (it is $($trees[0])). Rebuild from the current tree."
        }
    }
    return [pscustomobject]@{
        Commit = $commits[0]
        Tree   = $trees[0]
    }
}

# Local identity only. git is not on the remote trust boundary; it names the
# commit and tree this working copy is on so Assert-HeadBinding has something
# to refuse a stale binary against.
function Get-HeadIdentity {
    param([Parameter(Mandatory)][string] $Root)

    $git = @(Get-Command -Name @('git.exe', 'git') -CommandType Application -ErrorAction SilentlyContinue) |
        Select-Object -First 1
    if ($null -eq $git) {
        throw 'git was not found; a publish binds the artifact to HEAD'
    }
    $commit = (Invoke-BoundedProcess -FilePath $git.Source -ArgumentList @('-C', $Root, 'rev-parse', 'HEAD') `
        -What 'reading HEAD' -TimeoutSeconds 15).StandardOutput.Trim()
    $tree = (Invoke-BoundedProcess -FilePath $git.Source -ArgumentList @('-C', $Root, 'rev-parse', 'HEAD^{tree}') `
        -What 'reading HEAD tree' -TimeoutSeconds 15).StandardOutput.Trim()
    return [pscustomobject]@{
        Commit = (Assert-GitObjectId -Value $commit -What 'HEAD commit')
        Tree   = (Assert-GitObjectId -Value $tree -What 'HEAD tree')
    }
}

function Assert-ArtifactSize {
    param(
        [Parameter(Mandatory)][long] $Size,
        [Parameter(Mandatory)][string] $What
    )

    if ($Size -lt 0) {
        throw "$What is $Size bytes, which is not a size"
    }
    if ($Size -gt $script:MaxArtifactBytes) {
        throw "$What is $Size bytes, above the $($script:MaxArtifactBytes)-byte client ceiling"
    }
    return $Size
}

# The manifest the publish writes, and therefore the exact document the live
# endpoint has to return.
function New-ReleaseManifest {
    param(
        [Parameter(Mandatory)][string] $Version,
        [Parameter(Mandatory)][string] $FileName,
        [Parameter(Mandatory)][string] $Sha256,
        [Parameter(Mandatory)][long] $Size
    )

    Assert-Grammar -Value $Version -Pattern $script:VersionGrammar -What 'version' | Out-Null
    Assert-Grammar -Value $FileName -Pattern $script:ArtifactNameGrammar -What 'artifact name' | Out-Null
    Assert-Grammar -Value $Sha256 -Pattern $script:Sha256Grammar -What 'artifact hash' | Out-Null
    Assert-ArtifactSize -Size $Size -What 'the artifact' | Out-Null

    return [ordered]@{
        schemaVersion = 1
        version       = $Version
        artifacts     = @(
            [ordered]@{
                kind         = 'portable'
                architecture = 'x64'
                url          = "https://files.blockitall.us/aitierlist/releases/$Version/$FileName"
                sha256       = $Sha256
                size         = $Size
            }
        )
    }
}

# The exact bytes of the manifest that goes up, and its digest.
#
# UTF-8 without a BOM, so what is hashed here is what a client parses. The bytes
# are produced once and everything downstream - the upload, the staged check,
# the destination-local check and the final check - is about *these* bytes.
function New-ReleaseManifestBytes {
    param([Parameter(Mandatory)] $Manifest)

    $text = $Manifest | ConvertTo-Json -Depth 5
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($text)
    $sha = [System.Security.Cryptography.SHA256]::HashData($bytes)
    return [pscustomobject]@{
        Text   = $text
        Bytes  = $bytes
        Size   = [long]$bytes.Length
        Sha256 = ([System.BitConverter]::ToString($sha) -replace '-', '').ToLowerInvariant()
    }
}

# ------------------------------------------------ the live manifest, exactly

# The strict JSON boundary.
#
# The old form read fields off whatever `ConvertFrom-Json` produced and compared
# them as strings. That blurs every distinction that matters at a trust
# boundary: `"1"` and `1` and `1.0` and `true` all stringify to something a
# comparison accepted, a property lookup is case-insensitive so `Version` passed
# as `version`, extra fields were invisible, a duplicate key silently won, and a
# top-level array of one element was unwrapped into the object it contained.
#
# So the document is parsed with System.Text.Json and inspected as JSON: value
# kinds, exact property names in the exact case, no extras and no duplicates,
# numbers that are integral and in range, and every value compared with the
# locally constructed expectation.
function Get-JsonDocumentType {
    if (-not ('System.Text.Json.JsonDocument' -as [type])) {
        try { Add-Type -AssemblyName 'System.Text.Json' -ErrorAction Stop } catch { }
    }
    $type = 'System.Text.Json.JsonDocument' -as [type]
    if ($null -eq $type) {
        throw 'System.Text.Json is required to validate the live manifest and is not available'
    }
    return $type
}

# The property names of a JSON object, in document order, rejecting duplicates
# and anything outside the exact case-sensitive set.
function Assert-JsonObjectShape {
    param(
        [Parameter(Mandatory)] $Element,
        [Parameter(Mandatory)][string[]] $Required,
        [Parameter(Mandatory)][string] $What
    )

    if ($Element.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
        throw "$What is not a JSON object (it is $($Element.ValueKind))"
    }
    $seen = [System.Collections.Generic.List[string]]::new()
    foreach ($property in $Element.EnumerateObject()) {
        $name = $property.Name
        if ($seen.Contains($name)) { throw "$What carries a duplicate '$name' field" }
        $seen.Add($name)
    }
    foreach ($name in $seen) {
        if ($Required -cnotcontains $name) {
            throw "$What carries an unexpected field '$name'"
        }
    }
    foreach ($name in $Required) {
        if ($seen -cnotcontains $name) {
            throw "$What is missing the '$name' field"
        }
    }
    if ($seen.Count -ne $Required.Count) {
        throw "$What does not carry exactly $($Required.Count) fields"
    }
    return $true
}

# One case-sensitively named property of a JSON object. `TryGetProperty` is
# case-sensitive, but it also answers with the *first* match, so the duplicate
# check above has to have run first.
function Get-JsonProperty {
    param(
        [Parameter(Mandatory)] $Element,
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][string] $What
    )

    foreach ($property in $Element.EnumerateObject()) {
        if ($property.Name -ceq $Name) { return $property.Value }
    }
    throw "$What is missing the '$Name' field"
}

function Assert-JsonString {
    param(
        [Parameter(Mandatory)] $Element,
        [Parameter(Mandatory)][AllowEmptyString()][string] $Expected,
        [Parameter(Mandatory)][string] $What
    )

    if ($Element.ValueKind -ne [System.Text.Json.JsonValueKind]::String) {
        throw "$What is not a JSON string (it is $($Element.ValueKind))"
    }
    $value = $Element.GetString()
    if ($value -cne $Expected) {
        throw "$What is '$value', expected '$Expected'"
    }
    return $value
}

# A JSON number that is integral as written, representable, inside a declared
# range, and exactly the expected value.
#
# `1.0` and `1e0` are rejected rather than rounded: this is a boundary, and a
# document whose numbers are not in the canonical integral form the publish
# wrote is not the document the publish wrote.
function Assert-JsonIntegral {
    param(
        [Parameter(Mandatory)] $Element,
        [Parameter(Mandatory)][long] $Expected,
        [Parameter(Mandatory)][long] $Minimum,
        [Parameter(Mandatory)][long] $Maximum,
        [Parameter(Mandatory)][string] $What
    )

    if ($Element.ValueKind -ne [System.Text.Json.JsonValueKind]::Number) {
        throw "$What is not a JSON number (it is $($Element.ValueKind))"
    }
    $raw = $Element.GetRawText()
    if ($raw -cnotmatch '^-?(0|[1-9][0-9]*)$') {
        throw "$What is '$raw', which is not an integral JSON number"
    }
    try { $value = [long]::Parse($raw, [System.Globalization.CultureInfo]::InvariantCulture) }
    catch { throw "$What is '$raw', which is not representable as a 64-bit integer" }
    if ($value -lt $Minimum -or $value -gt $Maximum) {
        throw "$What is $value, outside the accepted range $Minimum..$Maximum"
    }
    if ($value -ne $Expected) {
        throw "$What is $value, expected $Expected"
    }
    return $value
}

# Compare a fetched manifest against the one that was published, as JSON.
#
# Takes the raw body, not something a shell of a parser already reshaped:
# `ConvertFrom-Json` collapses a one-element array, folds property case and
# hides duplicates, so handing it the result of that would be validating the
# wrong document.
function Assert-LiveManifest {
    param(
        [Parameter(Mandatory)][AllowNull()][AllowEmptyString()] $Json,
        [Parameter(Mandatory)] $Expected
    )

    if ($null -eq $Json) { throw 'the live manifest is empty' }
    $raw = [string]$Json
    if ([string]::IsNullOrWhiteSpace($raw)) { throw 'the live manifest is empty' }

    $expectedArtifacts = @($Expected['artifacts'])
    if ($expectedArtifacts.Count -ne 1) {
        throw 'the expected manifest does not carry exactly one artifact'
    }
    $expectedArtifact = $expectedArtifacts[0]

    $documentType = Get-JsonDocumentType
    $options = [System.Text.Json.JsonDocumentOptions]::new()
    $options.AllowTrailingCommas = $false
    $options.CommentHandling = [System.Text.Json.JsonCommentHandling]::Disallow
    $options.MaxDepth = 8
    try { $document = $documentType::Parse($raw, $options) }
    catch { throw "the live manifest is not valid JSON: $($_.Exception.Message)" }

    try {
        $root = $document.RootElement
        # A top level that is anything but one object - an array (including a
        # one-element one), a scalar, `null` - is refused here rather than being
        # unwrapped into whatever it contains.
        if ($root.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
            throw "the live manifest is not a JSON object (it is $($root.ValueKind))"
        }
        Assert-JsonObjectShape -Element $root -What 'the live manifest' `
            -Required @('schemaVersion', 'version', 'artifacts') | Out-Null

        Assert-JsonIntegral -Element (Get-JsonProperty -Element $root -Name 'schemaVersion' -What 'the live manifest') `
            -Expected ([long]$Expected['schemaVersion']) -Minimum 1 -Maximum 1000 `
            -What 'live manifest schemaVersion' | Out-Null
        Assert-JsonString -Element (Get-JsonProperty -Element $root -Name 'version' -What 'the live manifest') `
            -Expected ([string]$Expected['version']) -What 'live manifest version' | Out-Null

        $artifacts = Get-JsonProperty -Element $root -Name 'artifacts' -What 'the live manifest'
        if ($artifacts.ValueKind -ne [System.Text.Json.JsonValueKind]::Array) {
            throw "live manifest artifacts is not a JSON array (it is $($artifacts.ValueKind))"
        }
        if ($artifacts.GetArrayLength() -ne 1) {
            throw "live manifest carries $($artifacts.GetArrayLength()) artifacts, expected exactly one"
        }
        $artifact = $null
        foreach ($item in $artifacts.EnumerateArray()) { $artifact = $item; break }
        if ($null -eq $artifact) { throw 'live manifest carries no artifact' }
        Assert-JsonObjectShape -Element $artifact -What 'the live manifest artifact' `
            -Required @('kind', 'architecture', 'url', 'sha256', 'size') | Out-Null

        foreach ($field in @('kind', 'architecture', 'url', 'sha256')) {
            Assert-JsonString -Element (Get-JsonProperty -Element $artifact -Name $field -What 'the live manifest artifact') `
                -Expected ([string]$expectedArtifact[$field]) -What "live manifest artifact $field" | Out-Null
        }
        Assert-JsonIntegral -Element (Get-JsonProperty -Element $artifact -Name 'size' -What 'the live manifest artifact') `
            -Expected ([long]$expectedArtifact['size']) -Minimum 0 -Maximum $script:MaxArtifactBytes `
            -What 'live manifest artifact size' | Out-Null
        return $true
    }
    finally {
        $document.Dispose()
    }
}

# An accepted HTTP status for the artifact probe. A status alone is not a proof
# of the object - a 206 answers a one-byte Range without saying what was served.
# Assert-LiveArtifact is the proof; this is only the status gate in front of it.
function Assert-AcceptedStatus {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Code)

    if ($Code -cne '200' -and $Code -cne '206') {
        throw "the live artifact returned HTTP $Code"
    }
    return $Code
}

# The object the endpoint actually served, as a size and a digest. HTTP 200
# alone proves nothing: a stale, truncated, corrupt or misrouted body still
# answers 200 to a one-byte Range. 206 is a Range answer, not the artifact.
function Assert-LiveArtifact {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Code,
        [Parameter(Mandatory)][long] $Size,
        [Parameter(Mandatory)][AllowEmptyString()][string] $Sha256,
        [Parameter(Mandatory)][long] $ExpectedSize,
        [Parameter(Mandatory)][string] $ExpectedSha256
    )

    if ($Code -cne '200') {
        throw "the live artifact returned HTTP $Code"
    }
    Assert-ArtifactSize -Size $ExpectedSize -What 'the published artifact' | Out-Null
    Assert-ArtifactSize -Size $Size -What 'the live artifact' | Out-Null
    if ($Size -ne $ExpectedSize) {
        throw "the live artifact is $Size bytes, expected $ExpectedSize"
    }
    Assert-Grammar -Value $ExpectedSha256 -Pattern $script:Sha256Grammar -What 'published artifact hash' | Out-Null
    if ($Sha256 -cnotmatch $script:Sha256Grammar) {
        throw "the live artifact hash '$Sha256' is not a SHA-256"
    }
    if ($Sha256 -cne $ExpectedSha256) {
        throw "the live artifact hashes $Sha256, expected $ExpectedSha256"
    }
    return $true
}

# The version this publish is allowed to be.
#
# `-Version` is pinned to Cargo.toml, not defaulted to it. Any other value
# would publish the binary that is actually in
# target\release - which was built from Cargo.toml's version and carries its
# stamp - under a name it is not, so the release would report one version and
# call itself another, and every client would re-offer the same update forever.
# The stamp check downstream catches that only after the artifact has been
# named, and only because the two happen to be the same string; this refuses
# the disagreement itself.
function Assert-RequestedVersion {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Requested,
        [Parameter(Mandatory)][string] $Manifest
    )

    if ([string]::IsNullOrEmpty($Requested)) { return $Manifest }
    if ($Requested -cne $Manifest) {
        throw "-Version $Requested does not match Cargo.toml ($Manifest). The binary in target\release was built from Cargo.toml; bump it there and rebuild."
    }
    return $Manifest
}

function ConvertFrom-ReleaseVersion {
    param([Parameter(Mandatory)][string] $Version)

    Assert-Grammar -Value $Version -Pattern $script:VersionGrammar -What 'version' | Out-Null
    $numeric = $Version
    $pre = $null
    $hyphen = $Version.IndexOf([char]'-')
    if ($hyphen -ge 0) {
        $numeric = $Version.Substring(0, $hyphen)
        $pre = $Version.Substring($hyphen + 1).Split('.')
    }
    $parts = $numeric.Split('.')
    $invariant = [System.Globalization.CultureInfo]::InvariantCulture
    return [pscustomobject]@{
        Major = [uint64]::Parse($parts[0], $invariant)
        Minor = [uint64]::Parse($parts[1], $invariant)
        Patch = [uint64]::Parse($parts[2], $invariant)
        Pre   = $pre
    }
}

# SemVer precedence over the three-component-plus-optional-prerelease grammar.
# A prerelease is older than the same numeric triple without one, so 0.8.4-rc.1
# cannot replace 0.8.4 on the live channel, and 0.8.3 cannot replace 0.8.4.
function Compare-ReleaseVersion {
    param(
        [Parameter(Mandatory)][string] $Left,
        [Parameter(Mandatory)][string] $Right
    )

    $a = ConvertFrom-ReleaseVersion -Version $Left
    $b = ConvertFrom-ReleaseVersion -Version $Right
    foreach ($name in @('Major', 'Minor', 'Patch')) {
        if ($a.$name -lt $b.$name) { return -1 }
        if ($a.$name -gt $b.$name) { return 1 }
    }
    $aPre = $a.Pre
    $bPre = $b.Pre
    if ($null -eq $aPre -and $null -eq $bPre) { return 0 }
    if ($null -eq $aPre) { return 1 }
    if ($null -eq $bPre) { return -1 }
    $limit = [Math]::Max($aPre.Length, $bPre.Length)
    for ($i = 0; $i -lt $limit; $i++) {
        if ($i -ge $aPre.Length) { return -1 }
        if ($i -ge $bPre.Length) { return 1 }
        $x = $aPre[$i]
        $y = $bPre[$i]
        $xNum = 0UL
        $yNum = 0UL
        $xIsNum = [uint64]::TryParse($x, [System.Globalization.NumberStyles]::None, [System.Globalization.CultureInfo]::InvariantCulture, [ref]$xNum)
        $yIsNum = [uint64]::TryParse($y, [System.Globalization.NumberStyles]::None, [System.Globalization.CultureInfo]::InvariantCulture, [ref]$yNum)
        if ($xIsNum -and $yIsNum) {
            if ($xNum -lt $yNum) { return -1 }
            if ($xNum -gt $yNum) { return 1 }
            continue
        }
        if ($xIsNum -and -not $yIsNum) { return -1 }
        if (-not $xIsNum -and $yIsNum) { return 1 }
        $ord = [string]::CompareOrdinal($x, $y)
        if ($ord -ne 0) { return [Math]::Sign($ord) }
    }
    return 0
}

function Assert-PublishedVersion {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Value)

    if ($Value -cnotmatch $script:PublishedVersionGrammar) {
        throw "the published version '$Value' is not an answer this publish understands"
    }
    return $Value
}

function Get-PublishedManifestVersion {
    param([Parameter(Mandatory)][AllowNull()][AllowEmptyString()] $Json)

    if ($null -eq $Json) { throw 'the published manifest is empty' }
    $raw = [string]$Json
    if ([string]::IsNullOrWhiteSpace($raw)) { throw 'the published manifest is empty' }

    $documentType = Get-JsonDocumentType
    $options = [System.Text.Json.JsonDocumentOptions]::new()
    $options.AllowTrailingCommas = $false
    $options.CommentHandling = [System.Text.Json.JsonCommentHandling]::Disallow
    $options.MaxDepth = 8
    try { $document = $documentType::Parse($raw, $options) }
    catch { throw "the published manifest is not valid JSON: $($_.Exception.Message)" }

    try {
        $root = $document.RootElement
        if ($root.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
            throw "the published manifest is not a JSON object (it is $($root.ValueKind))"
        }
        $element = Get-JsonProperty -Element $root -Name 'version' -What 'the published manifest'
        if ($element.ValueKind -ne [System.Text.Json.JsonValueKind]::String) {
            throw "published manifest version is not a JSON string (it is $($element.ValueKind))"
        }
        return (Assert-PublishedVersion -Value $element.GetString())
    }
    finally {
        $document.Dispose()
    }
}

# The live channel only moves forward. Hash-and-size CAS cannot see that an
# older checkout is rewriting update.json onto a superseded release: the
# previous document is a different hash, so the transaction treats it as the
# expected predecessor and promotes it. Comparing the two version strings is
# the check CAS is not.
function Assert-ReleaseMonotonic {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Published,
        [Parameter(Mandatory)][string] $Candidate
    )

    Assert-Grammar -Value $Candidate -Pattern $script:VersionGrammar -What 'version' | Out-Null
    $live = Assert-PublishedVersion -Value $Published
    if ($live -ceq 'absent') { return $Candidate }
    if ((Compare-ReleaseVersion -Left $Candidate -Right $live) -lt 0) {
        throw "candidate $Candidate is older than the published $live. The live channel only moves forward."
    }
    return $Candidate
}

function New-PublishedVersionScript {
    param([Parameter(Mandatory)][string] $WebRoot)

    Assert-Grammar -Value $WebRoot -Pattern $script:WebRootGrammar -What 'web root' | Out-Null
    $template = @'
if [ -f '@MANIFEST@' ]; then
    awk -F '"' '$2 == "version" { print $4; exit }' '@MANIFEST@'
else
    printf 'absent\n'
fi
'@
    return (Assert-NoPlaceholders -Script ($template.Replace('@MANIFEST@', "$WebRoot/update.json")))
}

# Whether a version that is already published may be published again.
#
# A published version is immutable. Anyone may already have installed it, and
# the whole update mechanism assumes that a version number identifies one set of
# bytes: replacing them behind it produces a machine reporting a version whose
# artifact it does not have, and a bug report nobody can reproduce.
#
# `-Force` is not permission to overwrite it. What -Force is *for* is repairing
# a publish that failed after the artifact went up - a manifest that was never
# written, a live check that could not run - and that is an idempotent
# republication of the identical bytes. Same hash goes through; a different
# hash is refused whatever the flag says. It is also not permission
# to point the live channel at an older release: Assert-ReleaseMonotonic
# refuses a backward candidate whether this flag is set or not.
#
# This answer is advisory: it decides which transaction to attempt, and the
# server rechecks the same facts under the publish lock before anything moves.
function Assert-Republishable {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Exists,
        [Parameter(Mandatory)][AllowEmptyString()][string] $RemoteSha,
        [Parameter(Mandatory)][string] $LocalSha,
        [Parameter(Mandatory)][string] $Version,
        [switch] $Force
    )

    if ($Exists -cne 'yes' -and $Exists -cne 'no') {
        throw "the release existence check returned an unexpected answer: '$Exists'"
    }
    if ($Exists -ceq 'no') { return 'new' }
    if ($RemoteSha -cne $LocalSha) {
        throw "$Version is already published with different bytes (remote $RemoteSha, local $LocalSha). A published version is immutable - bump the version and rebuild. -Force does not override this."
    }
    if (-not $Force) {
        throw "$Version is already published with these exact bytes. Pass -Force to republish them idempotently (which only rewrites the manifest)."
    }
    return 'identical'
}

# ----------------------------------------------- the artifact this is, exactly

# Every DLL the reviewed release imports, normal and delayed.
#
# An exact list, not a policy. Everything AITIERLIST needs at run time is either in
# the image or in System32 on every supported Windows: the C runtime is static,
# rustls/aws-lc is static, zstd is static, and the GPU renderer uses System32's
# DXGI while loading the D3D12 and Vulkan backends at run time.
# So a name that is not on this list is a dependency that was not there when
# the release was reviewed, and it is the interesting kind of change: a machine
# without it does not start the program, and a machine with a *different* file
# of that name beside the executable runs that instead.
#
# API sets are named individually rather than wildcarded. `api-ms-win-*` is not
# a trust boundary - it is a naming convention - and accepting the pattern would
# accept any future one silently.
$script:AllowedImportDlls = @(
    'advapi32.dll',
    'api-ms-win-core-synch-l1-2-0.dll',
    'bcryptprimitives.dll',
    'combase.dll',
    'crypt32.dll',
    'dwmapi.dll',
    'dxgi.dll',
    'gdi32.dll',
    'imm32.dll',
    'kernel32.dll',
    'ntdll.dll',
    'ole32.dll',
    'oleaut32.dll',
    'opengl32.dll',
    'setupapi.dll',
    'shell32.dll',
    'shlwapi.dll',
    'uiautomationcore.dll',
    'user32.dll',
    'userenv.dll',
    'uxtheme.dll',
    'ws2_32.dll'
)

function Get-AllowedImportDlls { return $script:AllowedImportDlls }

# Families worth naming when they turn up, so a failure says what went wrong
# rather than only that something did. The exact allowlist above is still what
# decides; these only choose the sentence.
$script:ImportDiagnostics = @(
    @{ Pattern = '^(vcruntime|msvcp|msvcr|concrt|api-ms-win-crt-)';
       Why     = 'a dynamically linked Visual C++ / UCRT runtime - the release must be built with +crt-static, or it will not start on a machine without the redistributable' },
    @{ Pattern = '^(libgcc|libstdc\+\+|libwinpthread|msys-|cygwin1)';
       Why     = 'a MinGW/MSYS/Cygwin runtime - a Rust release binary must not acquire one' },
    @{ Pattern = '^(libcrypto|libssl|awslc|aws-lc)';
       Why     = 'a dynamic TLS/crypto library - aws-lc must be linked statically, or the updater depends on a DLL nobody ships' },
    @{ Pattern = '^(libzstd|zstd)';
       Why     = 'a dynamic zstd - the embedded payload decompressor must be static' },
    @{ Pattern = '^(vulkan-|vulkan1|libshaderc|shaderc|spirv|dxcompiler|dxil|d3dcompiler|nvapi|amd_ags|atiadl|igd)';
       Why     = 'a graphics runtime, shader compiler or vendor library resolved at load time - these belong behind a delay-loaded system loader, never as a hard dependency' },
    @{ Pattern = '^(avcodec|avformat|avutil|avfilter|avdevice|swscale|swresample|postproc|ffmpeg|ffprobe)';
       Why     = 'an FFmpeg-family DLL - AITIERLIST ships ffmpeg.exe as an authenticated payload and must never link one' }
)

# --------------------------------------------------- bounded PE arithmetic

# Every offset, size and span below is computed in unsigned 64-bit and checked
# before it is used. A PE field is 32 bits of attacker-controlled data: adding
# two of them in a type that wraps, or handing the result to an array index
# that is narrower, is how a parser is made to read the wrong bytes and answer
# confidently about them.
function Assert-UnsignedField {
    param(
        [Parameter(Mandatory)][long] $Value,
        [Parameter(Mandatory)][string] $What
    )

    if ($Value -lt 0) {
        throw "$What is $Value, which is outside the supported unsigned range"
    }
    if ($Value -gt 4294967295) {
        throw "$What is $Value, which is larger than a 32-bit PE field"
    }
    return [uint64]$Value
}

function Add-Checked {
    param(
        [Parameter(Mandatory)][uint64] $Left,
        [Parameter(Mandatory)][uint64] $Right,
        [Parameter(Mandatory)][string] $What
    )

    if ($Right -gt ([uint64]::MaxValue - $Left)) {
        throw "$What overflows the 64-bit range this parser computes in"
    }
    return [uint64]($Left + $Right)
}

function ConvertTo-BoundedIndex {
    param(
        [Parameter(Mandatory)][uint64] $Value,
        [Parameter(Mandatory)][string] $What
    )

    if ($Value -gt ([uint64][int]::MaxValue)) {
        throw "$What is outside the supported index range"
    }
    return [int]$Value
}

# The sections that actually have bytes in the file, with both spans.
#
# A section's file-backed RVA span is min(VirtualSize, SizeOfRawData): the tail
# beyond SizeOfRawData is zero-fill that exists only once the image is mapped,
# and reading it out of the file reads whatever the *next* section put there.
# The PE-defined zero-VirtualSize case (object files and some linkers) is the
# one exception and may use SizeOfRawData.
function Get-FileBackedSections {
    param(
        [Parameter(Mandatory)] $Headers,
        [Parameter(Mandatory)][long] $Length,
        [Parameter(Mandatory)][string] $Path
    )

    $sections = @()
    foreach ($section in $Headers.SectionHeaders) {
        $name = "$($section.Name)"
        $rawSize = Assert-UnsignedField -Value ([long]$section.SizeOfRawData) -What "$Path section $name SizeOfRawData"
        if ($rawSize -eq 0) { continue }
        $rawStart = Assert-UnsignedField -Value ([long]$section.PointerToRawData) -What "$Path section $name PointerToRawData"
        $rawEnd = Add-Checked -Left $rawStart -Right $rawSize -What "$Path section $name raw span"
        if ($rawEnd -gt [uint64]$Length) {
            throw "$Path has a section ($name) that is not inside the file"
        }
        $virtualSize = Assert-UnsignedField -Value ([long]$section.VirtualSize) -What "$Path section $name VirtualSize"
        $backed = if ($virtualSize -eq 0) { $rawSize } else { [Math]::Min($virtualSize, $rawSize) }
        $rva = Assert-UnsignedField -Value ([long]$section.VirtualAddress) -What "$Path section $name VirtualAddress"
        $rvaEnd = Add-Checked -Left $rva -Right $backed -What "$Path section $name RVA span"
        $sections += [pscustomobject]@{
            Name     = $name
            RawStart = $rawStart
            RawEnd   = $rawEnd
            Rva      = $rva
            RvaEnd   = $rvaEnd
            Backed   = $backed
        }
    }
    if ($sections.Count -eq 0) { throw "$Path has no mapped sections" }

    # No two sections may claim the same file bytes, and no two may claim the
    # same address space. Either overlap is how one set of bytes is made to be
    # read as two different things - a name that resolves inside one section
    # while the walk that reached it believed it was inside another.
    $byRaw = @($sections | Sort-Object -Property RawStart)
    for ($index = 1; $index -lt $byRaw.Count; $index++) {
        if ($byRaw[$index].RawStart -lt $byRaw[$index - 1].RawEnd) {
            throw "$Path has overlapping sections ($($byRaw[$index - 1].Name) and $($byRaw[$index].Name))"
        }
    }
    $byRva = @($sections | Where-Object { $_.Backed -gt 0 } | Sort-Object -Property Rva)
    for ($index = 1; $index -lt $byRva.Count; $index++) {
        if ($byRva[$index].Rva -lt $byRva[$index - 1].RvaEnd) {
            throw "$Path has sections with overlapping addresses ($($byRva[$index - 1].Name) and $($byRva[$index].Name))"
        }
    }
    return , $sections
}

# An RVA resolved to a file offset, together with the end of the file-backed
# data of the section that contains it.
#
# The second half is the point. A name or a descriptor array reached through an
# RVA lives in exactly one section, and every read that follows must stop at
# that section's end - not at the end of the image. Without it a string can run
# through a section boundary, across a gap, and into unrelated data, and the
# parser reports whatever it finds there as a DLL name.
function Resolve-FileBackedRva {
    param(
        [Parameter(Mandatory)] $Sections,
        [Parameter(Mandatory)][uint64] $Rva,
        [Parameter(Mandatory)][string] $What
    )

    foreach ($section in $Sections) {
        if ($section.Backed -eq 0) { continue }
        if ($Rva -ge $section.Rva -and $Rva -lt $section.RvaEnd) {
            $offset = Add-Checked -Left $section.RawStart -Right ($Rva - $section.Rva) -What $What
            return [pscustomobject]@{
                Offset       = $offset
                SectionEnd   = $section.RawEnd
                SectionStart = $section.RawStart
                Section      = $section.Name
                Backed       = $section.Backed
            }
        }
    }
    throw "$What is not inside any section's file data"
}

# An ASCII DLL basename read out of an import descriptor.
#
# The name is a NUL-terminated byte string at an RVA the file chose. Everything
# about it is therefore attacker-controlled in the case that matters, so this
# reads it as bytes: bounded by the section it lives in, terminated, non-empty,
# printable ASCII only, and a bare file name rather than any kind of path.
function Read-ImportName {
    param(
        [Parameter(Mandatory)][byte[]] $Image,
        [Parameter(Mandatory)][int] $Offset,
        [Parameter(Mandatory)][int] $Limit,
        [Parameter(Mandatory)][string] $What
    )

    if ($Limit -gt $Image.Length) { $Limit = $Image.Length }
    if ($Offset -lt 0 -or $Offset -ge $Limit) {
        throw "$What names a DLL outside the file"
    }
    $end = $Offset
    while ($end -lt $Limit -and $Image[$end] -ne 0) { $end++ }
    if ($end -ge $Limit) { throw "$What has an unterminated DLL name" }
    $length = $end - $Offset
    if ($length -eq 0) { throw "$What has an empty DLL name" }
    if ($length -gt 255) { throw "$What has an implausibly long DLL name" }
    $bytes = New-Object byte[] $length
    [System.Array]::Copy($Image, $Offset, $bytes, 0, $length)
    foreach ($byte in $bytes) {
        if ($byte -lt 0x20 -or $byte -gt 0x7E) {
            throw "$What has a DLL name with a non-printable or non-ASCII byte"
        }
    }
    $name = [System.Text.Encoding]::ASCII.GetString($bytes)
    if ($name -match '[\\/:]' -or $name -match '^\s' -or $name -match '\s$') {
        throw "$What imports '$name', which is a path rather than a DLL name"
    }
    return $name.ToLowerInvariant()
}

# Whether every byte of a descriptor slot is zero.
#
# A terminator is the whole structure, not the two or three fields a walk
# happens to read. A descriptor with a zero name RVA and a non-zero anything
# else is an ordinary descriptor that must be validated or refused; treating it
# as the end of the array is how the rest of an import table is skipped.
function Test-ZeroSpan {
    param(
        [Parameter(Mandatory)][byte[]] $Image,
        [Parameter(Mandatory)][int] $Offset,
        [Parameter(Mandatory)][int] $Length
    )

    for ($index = 0; $index -lt $Length; $index++) {
        if ($Image[$Offset + $index] -ne 0) { return $false }
    }
    return $true
}

# A data directory resolved to the one file-backed section that contains all of
# it, with the walk's hard end.
function Resolve-Directory {
    param(
        [Parameter(Mandatory)] $Sections,
        [Parameter(Mandatory)][uint64] $Rva,
        [Parameter(Mandatory)][uint64] $Size,
        [Parameter(Mandatory)][string] $What
    )

    $span = Resolve-FileBackedRva -Sections $Sections -Rva $Rva -What $What
    $end = Add-Checked -Left $span.Offset -Right $Size -What "$What span"
    if ($end -gt $span.SectionEnd) {
        throw "$What runs past the end of the section that contains it"
    }
    return [pscustomobject]@{
        Offset     = $span.Offset
        End        = $end
        SectionEnd = $span.SectionEnd
        Section    = $span.Section
    }
}

# Every DLL an image imports, normal and delayed, as a sorted unique list.
#
# Duplicate descriptors are allowed - a linker may emit two for one DLL - and
# are reported once. A missing terminator is not allowed anywhere: an
# unterminated descriptor array is how a parser is walked off the end of the
# data it was given, and a terminator outside the directory's own declared size
# is not a terminator this walk ever reaches.
function Get-ImportedDlls {
    param(
        [Parameter(Mandatory)] $Headers,
        [Parameter(Mandatory)][byte[]] $Image,
        [Parameter(Mandatory)] $Sections
    )

    $names = [System.Collections.Generic.HashSet[string]]::new()

    # --- normal imports: IMAGE_IMPORT_DESCRIPTOR[], 20 bytes each, NameRva at +12
    $directory = $Headers.PEHeader.ImportTableDirectory
    $importRva = Assert-UnsignedField -Value ([long]$directory.RelativeVirtualAddress) -What 'the import directory RVA'
    $importSize = Assert-UnsignedField -Value ([long]$directory.Size) -What 'the import directory size'
    # Mandatory, and mandatory in both halves: a directory with an address and
    # no size describes nothing, and one with a size and no address points at
    # the image header.
    if ($importRva -eq 0 -or $importSize -eq 0) {
        throw 'the image has no import table; a Windows executable that imports nothing is not what this release is'
    }
    $importDirectory = Resolve-Directory -Sections $Sections -Rva $importRva -Size $importSize -What 'the import table'
    $found = $false
    $terminated = $false
    for ($index = 0; $index -lt 4096; $index++) {
        $entry = Add-Checked -Left $importDirectory.Offset -Right ([uint64]($index * 20)) -What 'an import descriptor'
        $entryEnd = Add-Checked -Left $entry -Right 20 -What 'an import descriptor'
        if ($entryEnd -gt $importDirectory.End) {
            throw 'the import table is not terminated inside the size its data directory declares'
        }
        $at = ConvertTo-BoundedIndex -Value $entry -What 'an import descriptor offset'
        if (Test-ZeroSpan -Image $Image -Offset $at -Length 20) { $terminated = $true; break }
        $nameRva = Assert-UnsignedField -Value ([long][System.BitConverter]::ToUInt32($Image, $at + 12)) -What 'an import name RVA'
        $nameSpan = Resolve-FileBackedRva -Sections $Sections -Rva $nameRva -What 'an imported DLL name'
        $null = $names.Add((Read-ImportName -Image $Image `
            -Offset (ConvertTo-BoundedIndex -Value $nameSpan.Offset -What 'an imported DLL name offset') `
            -Limit (ConvertTo-BoundedIndex -Value $nameSpan.SectionEnd -What 'an imported DLL name bound') `
            -What 'the import table'))
        $found = $true
    }
    if (-not $terminated) { throw 'the import table is not terminated' }
    if (-not $found) { throw 'the import table is empty' }

    # --- delay imports: ImgDelayDescr[], 32 bytes each, DllNameRVA at +4.
    # Optional as a whole: an image with neither an address nor a size has none.
    # A half-present directory is not "no delay imports", it is a malformed one.
    $delay = $Headers.PEHeader.DelayImportTableDirectory
    $delayRva = Assert-UnsignedField -Value ([long]$delay.RelativeVirtualAddress) -What 'the delay import directory RVA'
    $delaySize = Assert-UnsignedField -Value ([long]$delay.Size) -What 'the delay import directory size'
    if (($delayRva -eq 0) -ne ($delaySize -eq 0)) {
        throw 'the delay import data directory declares an address without a size, or a size without an address'
    }
    if ($delayRva -ne 0) {
        $delayDirectory = Resolve-Directory -Sections $Sections -Rva $delayRva -Size $delaySize -What 'the delay import table'
        $terminated = $false
        for ($index = 0; $index -lt 4096; $index++) {
            $entry = Add-Checked -Left $delayDirectory.Offset -Right ([uint64]($index * 32)) -What 'a delay import descriptor'
            $entryEnd = Add-Checked -Left $entry -Right 32 -What 'a delay import descriptor'
            if ($entryEnd -gt $delayDirectory.End) {
                throw 'the delay import table is not terminated inside the size its data directory declares'
            }
            $at = ConvertTo-BoundedIndex -Value $entry -What 'a delay import descriptor offset'
            if (Test-ZeroSpan -Image $Image -Offset $at -Length 32) { $terminated = $true; break }
            $attributes = [System.BitConverter]::ToUInt32($Image, $at)
            $nameField = Assert-UnsignedField -Value ([long][System.BitConverter]::ToUInt32($Image, $at + 4)) -What 'a delay import name address'
            if ($attributes -gt 1) {
                throw "the delay import table uses unsupported attributes 0x$($attributes.ToString('x'))"
            }
            if ($attributes -eq 0) {
                # Virtual addresses, not RVAs: subtract the image base to reach
                # one. Anything that does not land inside the image is refused.
                $base = [uint64]$Headers.PEHeader.ImageBase
                if ($nameField -lt $base) {
                    throw 'a delay import name address is below the image base'
                }
                $nameRva = [uint64]($nameField - $base)
            }
            else {
                $nameRva = $nameField
            }
            $nameSpan = Resolve-FileBackedRva -Sections $Sections -Rva $nameRva -What 'a delay-imported DLL name'
            $null = $names.Add((Read-ImportName -Image $Image `
                -Offset (ConvertTo-BoundedIndex -Value $nameSpan.Offset -What 'a delay-imported DLL name offset') `
                -Limit (ConvertTo-BoundedIndex -Value $nameSpan.SectionEnd -What 'a delay-imported DLL name bound') `
                -What 'the delay import table'))
        }
        if (-not $terminated) { throw 'the delay import table is not terminated' }
    }

    return @($names | Sort-Object)
}

# The whole gate, over an artifact that is already open.
#
# Takes the Stream, never a path. The publish holds one handle to the file it
# hashed, and everything downstream - this check and the upload - must be about
# *that object*: re-opening the name would let a rebuild, or anything else,
# substitute a different file between the hash and the check, or between the
# check and the upload. The stream's position is restored, so the caller's
# rewind still means what it did.
function Assert-ReleaseImage {
    param(
        [Parameter(Mandatory)][System.IO.Stream] $Stream,
        [string] $Path = 'the release binary'
    )

    if (-not $Stream.CanRead) { throw "$Path is not readable" }
    if (-not $Stream.CanSeek) { throw "$Path is not seekable" }
    $entry = $Stream.Position
    try {
        $null = $Stream.Seek(0, [System.IO.SeekOrigin]::Begin)
        # The bytes, read once from the held object. Everything below reads this
        # array; nothing re-reads the file.
        $image = New-Object byte[] $Stream.Length
        $read = 0
        while ($read -lt $image.Length) {
            $got = $Stream.Read($image, $read, $image.Length - $read)
            if ($got -le 0) { throw "$Path ended after $read of $($image.Length) bytes" }
            $read += $got
        }

        $null = $Stream.Seek(0, [System.IO.SeekOrigin]::Begin)
        # LeaveOpen: the caller still owns the stream and still needs it for the
        # upload.
        $reader = [System.Reflection.PortableExecutable.PEReader]::new(
            $Stream, [System.Reflection.PortableExecutable.PEStreamOptions]::LeaveOpen)
        try {
            $headers = $reader.PEHeaders
            if ($null -eq $headers.PEHeader) { throw "$Path has no PE optional header" }

            $magic = $headers.PEHeader.Magic
            if ($magic -ne [System.Reflection.PortableExecutable.PEMagic]::PE32Plus) {
                throw "$Path is $magic, not PE32+ (a 64-bit image)"
            }
            $machine = $headers.CoffHeader.Machine
            if ($machine -ne [System.Reflection.PortableExecutable.Machine]::Amd64) {
                throw "$Path is built for $machine, not Amd64"
            }
            # Compared against zero explicitly: a flags enum in a boolean
            # context is one implicit conversion away from being read backwards.
            $characteristics = [int] $headers.CoffHeader.Characteristics
            $executableBit = [int] [System.Reflection.PortableExecutable.Characteristics]::ExecutableImage
            $dllBit = [int] [System.Reflection.PortableExecutable.Characteristics]::Dll
            if (($characteristics -band $executableBit) -eq 0) {
                throw "$Path is not marked as an executable image"
            }
            if (($characteristics -band $dllBit) -ne 0) {
                throw "$Path is a DLL, not an executable"
            }

            # Sections: each mapped inside the file, none overlapping another in
            # the file, and none overlapping another in the address space.
            $sections = Get-FileBackedSections -Headers $headers -Length $image.Length -Path $Path

            # The load configuration directory, and the one field that matters.
            # 0x0800 is LOAD_LIBRARY_SEARCH_SYSTEM32, recorded in the image so
            # the loader resolves this executable's imports from System32 before
            # its first instruction runs. Without it, a same-named DLL beside
            # aitierlist.exe wins - and aitierlist.exe is what launches the bundled ffmpeg
            # out of a user-writable cache directory.
            $loadConfig = $headers.PEHeader.LoadConfigTableDirectory
            $configRva = Assert-UnsignedField -Value ([long]$loadConfig.RelativeVirtualAddress) -What "$Path load configuration RVA"
            $configSize = Assert-UnsignedField -Value ([long]$loadConfig.Size) -What "$Path load configuration size"
            if ($configRva -eq 0 -or $configSize -eq 0) {
                throw "$Path has no load configuration directory, so it carries no DependentLoadFlags"
            }
            if ($configSize -lt 0x50) {
                throw "$Path has a load configuration directory of $configSize bytes, which is too small to contain DependentLoadFlags"
            }
            # Bounded to the one file-backed section that contains all of it. A
            # structure that lives in a virtual-only zero-fill tail, or that
            # crosses out of its section into the next one, is not data this
            # file actually carries.
            $configDirectory = Resolve-Directory -Sections $sections -Rva $configRva -Size $configSize `
                -What "$Path load configuration directory"
            $configAt = ConvertTo-BoundedIndex -Value $configDirectory.Offset -What "$Path load configuration offset"
            # IMAGE_LOAD_CONFIG_DIRECTORY64: DependentLoadFlags is the WORD at
            # offset 0x4E, immediately after CSDVersion - the same constant
            # tools.rs's own parser uses.
            #
            # The structure's *declared* Size decides whether the field is part
            # of it at all: a directory that maps 0x50 bytes over a structure
            # that says it is 0x40 long would otherwise be read past its end.
            $declared = Assert-UnsignedField -Value ([long][System.BitConverter]::ToUInt32($image, $configAt)) `
                -What "$Path load configuration declared size"
            if ($declared -lt 0x50) {
                throw ("$Path has a load configuration structure declaring 0x{0:x} bytes, too few to contain DependentLoadFlags" -f $declared)
            }
            if ($declared -gt $configSize) {
                throw ("$Path has a load configuration structure declaring 0x{0:x} bytes, past the 0x{1:x} its data directory maps" -f $declared, $configSize)
            }
            # Every byte the read needs, file-backed, inside this one section.
            $through = Add-Checked -Left $configDirectory.Offset -Right 0x50 -What "$Path load configuration flags"
            if ($through -gt $configDirectory.End -or $through -gt $configDirectory.SectionEnd -or
                $through -gt [uint64]$image.Length) {
                throw "$Path has a load configuration structure whose DependentLoadFlags are not backed by file data"
            }
            $flags = [System.BitConverter]::ToUInt16($image, $configAt + 0x4E)
            if ($flags -ne 0x0800) {
                throw ("$Path records DependentLoadFlags 0x{0:x4}, not 0x0800 (LOAD_LIBRARY_SEARCH_SYSTEM32). " -f $flags) +
                      'Rebuild with the /DEPENDENTLOADFLAG:0x800 link argument build.rs emits.'
            }

            $imported = Get-ImportedDlls -Headers $headers -Image $image -Sections $sections
            $allowed = Get-AllowedImportDlls
            $unexpected = @($imported | Where-Object { $allowed -notcontains $_ })
            if ($unexpected.Count -gt 0) {
                $detail = foreach ($name in $unexpected) {
                    # Matched first, read second. Reading `.Why` off a pipeline
                    # that selected nothing is a property access on $null, which
                    # under StrictMode is a second, unrelated error inside the
                    # message that was meant to explain the first.
                    $match = @($script:ImportDiagnostics |
                        Where-Object { $name -match $_.Pattern } |
                        Select-Object -First 1)
                    if ($match.Count -gt 0) { "$name ($($match[0].Why))" } else { $name }
                }
                throw "$Path imports DLLs that are not on the reviewed list: $($detail -join '; ')"
            }
            return [pscustomobject]@{
                Machine            = "$machine"
                Magic              = "$magic"
                DependentLoadFlags = $flags
                Imports            = $imported
            }
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $null = $Stream.Seek($entry, [System.IO.SeekOrigin]::Begin)
    }
}

# --------------------------------------------- a bounded, noninteractive child

# The ssh arguments every call this script makes must carry.
#
# `-o BatchMode=yes` and `-o NumberOfPasswordPrompts=0` remove every prompt: a
# publish that stops to ask for a password is a publish that hangs forever in
# CI and, worse, one whose stdin is the bytes of a release. The connect and
# server-alive bounds mean a host that accepts a connection and then stops
# answering is a failure rather than a wait.
#
# `-T` (no pseudo-terminal) is on both, so nothing translates the byte stream.
# `-n` (stdin from /dev/null) is on control calls only: an upload's standard
# input *is* the payload, and redirecting it would send an empty file.
$script:SshCommonArguments = @(
    '-T',
    '-o', 'BatchMode=yes',
    '-o', 'NumberOfPasswordPrompts=0',
    '-o', 'ConnectTimeout=15',
    '-o', 'ServerAliveInterval=15',
    '-o', 'ServerAliveCountMax=4'
)
$script:SshUploadArguments = @($script:SshCommonArguments)
$script:SshControlArguments = @('-n') + $script:SshCommonArguments

function Get-SshUploadArguments { return $script:SshUploadArguments }
function Get-SshControlArguments { return $script:SshControlArguments }

# How long any one remote call may take, and how much it may say.
$script:DefaultProcessTimeoutSeconds = 900
$script:DefaultControlTimeoutSeconds = 120
$script:DefaultMaxOutputBytes = 1048576
$script:DefaultCleanupSeconds = 10
$script:DiagnosticCharacters = 2000

function Get-DefaultProcessTimeoutSeconds { return $script:DefaultProcessTimeoutSeconds }
function Get-DefaultControlTimeoutSeconds { return $script:DefaultControlTimeoutSeconds }
function Get-DefaultMaxOutputBytes { return $script:DefaultMaxOutputBytes }

# The first real message inside a faulted Task, without the AggregateException
# wrapper nobody wants to read.
function Get-TaskFailureMessage {
    param([Parameter(Mandatory)] $Task)

    $aggregate = $Task.Exception
    if ($null -eq $aggregate) { return 'the operation failed' }
    $flattened = $aggregate.Flatten()
    if ($flattened.InnerExceptions.Count -gt 0) { return $flattened.InnerExceptions[0].Message }
    return $flattened.Message
}

# What the child said, bounded. A failure message is a diagnostic, not a
# transcript: a child that emitted a megabyte before dying must not turn one
# thrown error into a megabyte of console.
function Format-ChildDiagnostics {
    param(
        [AllowEmptyString()][string] $StandardOutput = '',
        [AllowEmptyString()][string] $StandardError = '',
        [int] $MaxCharacters = 0
    )

    if ($MaxCharacters -le 0) { $MaxCharacters = $script:DiagnosticCharacters }
    $detail = (@($StandardOutput, $StandardError) -join "`n").Trim()
    if ([string]::IsNullOrEmpty($detail)) { return '' }
    if ($detail.Length -gt $MaxCharacters) {
        $detail = $detail.Substring(0, $MaxCharacters) + '... (truncated)'
    }
    return ": $detail"
}

# Run one child with a hard wall-clock deadline and hard output bounds.
#
# Everything a publish does to a remote host goes through here, and every one of
# the ways a child can stop a publisher forever is bounded:
#
#   * it never reads standard input       - the copy is asynchronous, and the
#                                           deadline covers it;
#   * it reads to EOF and then hangs      - the deadline covers the exit too;
#   * it floods standard output or error  - each pipe has its own byte bound and
#                                           they are drained concurrently, so
#                                           neither can fill a pipe the other is
#                                           blocked behind;
#   * it exits before reading             - the broken pipe surfaces as a copy
#                                           failure rather than a hang;
#   * it exits zero after a local failure - the local failure is the one that is
#                                           reported.
#
# On any failure the primary error is preserved, standard input is closed, the
# spawned tree is killed, and the reap is itself bounded: cleaning up must not
# become the new way to wait forever.
#
# ProcessStartInfo with ArgumentList, never a hand-quoted Arguments string and
# never a PowerShell text pipeline: the first re-quotes for a shell that is not
# there, and the second re-encodes bytes as text.
function Invoke-BoundedProcess {
    param(
        [Parameter(Mandatory)][string] $FilePath,
        [Parameter(Mandatory)][AllowEmptyCollection()][string[]] $ArgumentList,
        [Parameter(Mandatory)][string] $What,
        [System.IO.Stream] $InputStream,
        [int] $TimeoutSeconds = 0,
        [int] $MaxOutputBytes = 0,
        [int] $CleanupSeconds = 0
    )

    if ($TimeoutSeconds -le 0) { $TimeoutSeconds = $script:DefaultProcessTimeoutSeconds }
    if ($MaxOutputBytes -le 0) { $MaxOutputBytes = $script:DefaultMaxOutputBytes }
    if ($CleanupSeconds -le 0) { $CleanupSeconds = $script:DefaultCleanupSeconds }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    # Every argument added as its own element.
    foreach ($argument in $ArgumentList) { $startInfo.ArgumentList.Add($argument) }

    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $limit = [System.TimeSpan]::FromSeconds($TimeoutSeconds)
    $cancellation = [System.Threading.CancellationTokenSource]::new()
    $outSink = [System.IO.MemoryStream]::new()
    $errSink = [System.IO.MemoryStream]::new()
    $pipes = @(
        [pscustomobject]@{ Name = 'standard output'; Stream = $null; Sink = $outSink; Buffer = (New-Object byte[] 8192); Pending = $null; Done = $false },
        [pscustomobject]@{ Name = 'standard error'; Stream = $null; Sink = $errSink; Buffer = (New-Object byte[] 8192); Pending = $null; Done = $false }
    )
    $process = $null
    $copyTask = $null
    $stdinClosed = $false
    $failure = $null

    try {
        $process = [System.Diagnostics.Process]::Start($startInfo)

        # Both drains started before a single byte is written. A child writes
        # progress and errors while it reads; a producer that filled the pipe
        # and only then read them would deadlock.
        $pipes[0].Stream = $process.StandardOutput.BaseStream
        $pipes[1].Stream = $process.StandardError.BaseStream
        foreach ($pipe in $pipes) {
            $pipe.Pending = $pipe.Stream.ReadAsync($pipe.Buffer, 0, $pipe.Buffer.Length, $cancellation.Token)
        }

        if ($null -eq $InputStream) {
            # Nothing to send, so EOF immediately: a child waiting on standard
            # input it will never receive is a hang this function must not
            # create.
            try { $process.StandardInput.Close() } catch { }
            $stdinClosed = $true
        }
        else {
            $copyTask = $InputStream.CopyToAsync(
                $process.StandardInput.BaseStream, 81920, $cancellation.Token)
        }

        while ($true) {
            if ($clock.Elapsed -ge $limit) {
                $failure = "$What did not finish within $TimeoutSeconds seconds"
                break
            }
            $progressed = $false

            if ($null -ne $copyTask -and -not $stdinClosed -and $copyTask.IsCompleted) {
                $progressed = $true
                if ($copyTask.IsFaulted) {
                    $failure = "$What failed while sending the bytes: $(Get-TaskFailureMessage -Task $copyTask)"
                }
                elseif ($copyTask.IsCanceled) {
                    $failure = "$What was cancelled while sending the bytes"
                }
                else {
                    try { $process.StandardInput.BaseStream.Flush() }
                    catch { $failure = "$What failed while flushing the bytes: $($_.Exception.Message)" }
                }
                # Closed whatever happened: stdin's EOF is what tells `cat` to
                # finish, and a child that is still reading must be released
                # rather than left holding a pipe.
                try { $process.StandardInput.Close() }
                catch {
                    if ($null -eq $failure) {
                        $failure = "$What failed while closing the remote input: $($_.Exception.Message)"
                    }
                }
                $stdinClosed = $true
            }
            if ($null -ne $failure) { break }

            foreach ($pipe in $pipes) {
                if ($pipe.Done) { continue }
                if (-not $pipe.Pending.IsCompleted) { continue }
                $progressed = $true
                if ($pipe.Pending.IsFaulted) {
                    $failure = "$What failed while reading its $($pipe.Name): $(Get-TaskFailureMessage -Task $pipe.Pending)"
                    break
                }
                if ($pipe.Pending.IsCanceled) { $pipe.Done = $true; continue }
                $count = $pipe.Pending.Result
                if ($count -le 0) { $pipe.Done = $true; continue }
                if (($pipe.Sink.Length + $count) -gt $MaxOutputBytes) {
                    $failure = "$What wrote more than $MaxOutputBytes bytes to its $($pipe.Name)"
                    break
                }
                $pipe.Sink.Write($pipe.Buffer, 0, $count)
                $pipe.Pending = $pipe.Stream.ReadAsync($pipe.Buffer, 0, $pipe.Buffer.Length, $cancellation.Token)
            }
            if ($null -ne $failure) { break }

            $drained = $true
            foreach ($pipe in $pipes) { if (-not $pipe.Done) { $drained = $false } }
            if ($drained -and $stdinClosed -and $process.HasExited) { break }
            if (-not $progressed) { Start-Sleep -Milliseconds 25 }
        }

        $outText = [System.Text.Encoding]::UTF8.GetString($outSink.ToArray())
        $errText = [System.Text.Encoding]::UTF8.GetString($errSink.ToArray())
        if ($null -ne $failure) {
            throw ($failure + (Format-ChildDiagnostics -StandardOutput $outText -StandardError $errText))
        }
        # A child that exited zero never masks a local failure: the branch above
        # runs first, and this one is only reached when every local operation
        # succeeded.
        $exit = $process.ExitCode
        if ($exit -ne 0) {
            throw ("$What failed (exit $exit)" +
                (Format-ChildDiagnostics -StandardOutput $outText -StandardError $errText))
        }
        return [pscustomobject]@{
            ExitCode       = $exit
            StandardOutput = $outText
            StandardError  = $errText
        }
    }
    finally {
        try { $cancellation.Cancel() } catch { }
        if ($null -ne $process) {
            try { if (-not $stdinClosed) { $process.StandardInput.Close() } } catch { }
            # Only the process this function started, its whole tree, and never
            # in a way that hides the error that brought us here.
            try { if (-not $process.HasExited) { $process.Kill($true) } } catch { }
            try { $null = $process.WaitForExit([int]($CleanupSeconds * 1000)) } catch { }
            try { $process.Dispose() } catch { }
        }
        # Bounded, and observed: a faulted Task nobody looked at raises an
        # unobserved-exception event when it is finalized.
        foreach ($pipe in $pipes) {
            if ($null -ne $pipe.Pending) {
                try { $null = $pipe.Pending.Wait([int]($CleanupSeconds * 1000)) } catch { }
                try { $null = $pipe.Pending.Exception } catch { }
            }
        }
        if ($null -ne $copyTask) {
            try { $null = $copyTask.Wait([int]($CleanupSeconds * 1000)) } catch { }
            try { $null = $copyTask.Exception } catch { }
        }
        try { $outSink.Dispose() } catch { }
        try { $errSink.Dispose() } catch { }
        try { $cancellation.Dispose() } catch { }
    }
}

# ------------------------------------------------- an object-bound remote copy

# Stream an already-open, already-verified object to a remote file.
#
# `scp $exe host:...` is not object-bound: scp performs a *fresh* pathname
# resolution, and a same-user process can rename an ancestor directory between
# the verification and the upload while the original leaf handle stays open. The
# bytes that were checked and the bytes that go up would then be two different
# files. Handing the verified stream to ssh's standard input removes the second
# resolution entirely - there is no local pathname in the transfer at all.
function Send-StreamToRemoteFile {
    param(
        [Parameter(Mandatory)][string] $Ssh,
        [Parameter(Mandatory)][string] $RemoteHost,
        [Parameter(Mandatory)][string] $RemoteCommand,
        [Parameter(Mandatory)][System.IO.Stream] $InputStream,
        [string] $What = 'a remote write',
        [int] $TimeoutSeconds = 0,
        [int] $MaxOutputBytes = 0,
        # The arguments that come before the host, so a test can put a local
        # stand-in where ssh goes. Production never passes this: the
        # noninteractive upload set is the whole list, and publish.ps1 is
        # asserted not to override it.
        [string[]] $LeadingArguments = @()
    )

    if ($null -eq $LeadingArguments -or $LeadingArguments.Count -eq 0) {
        $LeadingArguments = Get-SshUploadArguments
    }
    $arguments = @($LeadingArguments) + @($RemoteHost, $RemoteCommand)
    $result = Invoke-BoundedProcess -FilePath $Ssh -ArgumentList $arguments -What $What `
        -InputStream $InputStream -TimeoutSeconds $TimeoutSeconds -MaxOutputBytes $MaxOutputBytes
    return $result.StandardError
}

# One remote command, bounded and noninteractive, with its standard input
# already at end of file.
function Invoke-RemoteCommand {
    param(
        [Parameter(Mandatory)][string] $Ssh,
        [Parameter(Mandatory)][string] $RemoteHost,
        [Parameter(Mandatory)][string] $Command,
        [string] $What = 'a remote command',
        [int] $TimeoutSeconds = 0,
        [int] $MaxOutputBytes = 0,
        [string[]] $LeadingArguments = @()
    )

    if ($null -eq $LeadingArguments -or $LeadingArguments.Count -eq 0) {
        $LeadingArguments = Get-SshControlArguments
    }
    if ($TimeoutSeconds -le 0) { $TimeoutSeconds = $script:DefaultControlTimeoutSeconds }
    $arguments = @($LeadingArguments) + @($RemoteHost, $Command.Replace("`r`n", "`n"))
    $result = Invoke-BoundedProcess -FilePath $Ssh -ArgumentList $arguments -What $What `
        -TimeoutSeconds $TimeoutSeconds -MaxOutputBytes $MaxOutputBytes
    return $result.StandardOutput.Trim()
}

# ---------------------------------------------------- the remote transaction

# The snapshot of the currently published manifest this publish is transacting
# against: an exact hash and size, or an explicit absent sentinel.
#
# Anything else - a directory where the manifest should be, a truncated answer,
# a shell that said something conversational - is refused rather than guessed
# at. A publish that cannot state what it is replacing may not replace it.
function Assert-ManifestCas {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Snapshot)

    if ($Snapshot -cnotmatch $script:ManifestCasGrammar) {
        throw "the published manifest snapshot is not an answer this publish understands: '$Snapshot'"
    }
    return $Snapshot
}

function Assert-NoPlaceholders {
    param([Parameter(Mandatory)][string] $Script)

    if ($Script -cmatch '@[A-Z_]+@') {
        throw 'a remote script was built with an unreplaced placeholder'
    }
    return $Script
}

# The command that produces that snapshot.
function New-ManifestCasScript {
    param([Parameter(Mandatory)][string] $WebRoot)

    Assert-Grammar -Value $WebRoot -Pattern $script:WebRootGrammar -What 'web root' | Out-Null
    $template = @'
if [ -f '@MANIFEST@' ]; then
    printf '%s %s\n' "$(shasum -a 256 '@MANIFEST@' | cut -d' ' -f1)" "$(wc -c < '@MANIFEST@' | tr -d ' ')"
elif [ -e '@MANIFEST@' ]; then
    printf 'unusable\n'
else
    printf 'absent\n'
fi
'@
    return (Assert-NoPlaceholders -Script ($template.Replace('@MANIFEST@', "$WebRoot/update.json")))
}

# Copy each verified staged object into a hidden temporary that already lives in
# its own final destination directory, and answer with what landed there.
#
# This is the step that makes the publication atomic. A `mv` out of `$HOME` can
# cross a filesystem boundary, and a cross-device move is a copy followed by an
# unlink: the final name exists, partially written, while the copy is still
# running. A rename inside one directory cannot do that, so the bytes are put
# in that directory first - under a name nothing serves - and only renamed once
# they have been proven there.
function New-DestinationTempScript {
    param(
        [Parameter(Mandatory)][string] $WebRoot,
        [Parameter(Mandatory)][string] $RemoteDir,
        [Parameter(Mandatory)][string] $StagePath,
        [Parameter(Mandatory)][string] $FileName,
        [Parameter(Mandatory)][string] $ArtifactTemp,
        [Parameter(Mandatory)][string] $ManifestTemp
    )

    Assert-Grammar -Value $WebRoot -Pattern $script:WebRootGrammar -What 'web root' | Out-Null
    Assert-Grammar -Value $RemoteDir -Pattern $script:WebRootGrammar -What 'release directory' | Out-Null
    Assert-Grammar -Value $FileName -Pattern $script:ArtifactNameGrammar -What 'artifact name' | Out-Null
    Assert-Grammar -Value $ArtifactTemp -Pattern $script:WebRootGrammar -What 'artifact temporary' | Out-Null
    Assert-Grammar -Value $ManifestTemp -Pattern $script:WebRootGrammar -What 'manifest temporary' | Out-Null
    # The stage is the remote shell's own answer, already validated once when it
    # was created; re-checked here because it is interpolated again.
    Assert-Grammar -Value $StagePath -Pattern '^(/[^\r\n''"\\`$]+)+$' -What 'remote stage path' | Out-Null
    $template = @'
set -eu
sudo -n mkdir -p '@DIR@'
sudo -n mkdir -p '@WEB@'
sudo -n cp '@STAGE@/@FILE@' '@ATMP@'
sudo -n chmod 644 '@ATMP@'
sudo -n cp '@STAGE@/update.json' '@MTMP@'
sudo -n chmod 644 '@MTMP@'
printf '%s %s\n' "$(shasum -a 256 '@ATMP@' | cut -d' ' -f1)" "$(wc -c < '@ATMP@' | tr -d ' ')"
printf '%s %s\n' "$(shasum -a 256 '@MTMP@' | cut -d' ' -f1)" "$(wc -c < '@MTMP@' | tr -d ' ')"
'@
    $script = $template.
        Replace('@DIR@', $RemoteDir).
        Replace('@WEB@', $WebRoot).
        Replace('@STAGE@', $StagePath).
        Replace('@FILE@', $FileName).
        Replace('@ATMP@', $ArtifactTemp).
        Replace('@MTMP@', $ManifestTemp)
    return (Assert-NoPlaceholders -Script $script)
}

# The one transaction that publishes, as a single fail-closed remote script.
#
# Everything that can change the published tree happens under one server-side
# lock, after the compare-and-set snapshot has been rechecked, and in an order
# where no client can ever read a manifest that points at bytes which are not
# there:
#
#   acquire the lock (a directory: creating one is atomic). A lock older than
#     the stale window is the leftover of a killed publisher, not a holder,
#     and is removed once so the next mkdir can proceed; a fresh lock still
#     exits 11
#     recheck the manifest CAS snapshot        - a concurrent publisher aborts
#                                                this one rather than being
#                                                rolled back over
#     refuse a candidate older than the version currently in update.json
#     recheck both destination-local temporaries by hash and size
#     if the release is already there: it must be these exact bytes, and it is
#       then left completely alone - not renamed over, not chmod-ed, not
#       touched, so its inode and timestamps are what they were
#     otherwise: rename this run's temporary onto the absent final name. Same
#       directory, so the rename is atomic and no partial file ever wears the
#       published name
#     verify the final release again, where it will be served from
#     promote the manifest last, and verify it too
#   release the lock
#
# A trap releases the lock and removes this run's own temporaries on every exit
# path. It removes nothing else: not the stage, not another run's temporaries,
# and never anything published.
function New-PublishTransactionScript {
    param(
        [Parameter(Mandatory)][string] $WebRoot,
        [Parameter(Mandatory)][string] $RemoteDir,
        [Parameter(Mandatory)][string] $FileName,
        [Parameter(Mandatory)][string] $ArtifactTemp,
        [Parameter(Mandatory)][string] $ManifestTemp,
        [Parameter(Mandatory)][string] $ManifestCas,
        [Parameter(Mandatory)][string] $ArtifactSha,
        [Parameter(Mandatory)][long] $ArtifactSize,
        [Parameter(Mandatory)][string] $ManifestSha,
        [Parameter(Mandatory)][long] $ManifestSize,
        [Parameter(Mandatory)][ValidateSet('new', 'identical')][string] $Mode,
        [Parameter(Mandatory)][string] $CandidateVersion
    )

    Assert-Grammar -Value $WebRoot -Pattern $script:WebRootGrammar -What 'web root' | Out-Null
    Assert-Grammar -Value $RemoteDir -Pattern $script:WebRootGrammar -What 'release directory' | Out-Null
    Assert-Grammar -Value $FileName -Pattern $script:ArtifactNameGrammar -What 'artifact name' | Out-Null
    Assert-Grammar -Value $ArtifactTemp -Pattern $script:WebRootGrammar -What 'artifact temporary' | Out-Null
    Assert-Grammar -Value $ManifestTemp -Pattern $script:WebRootGrammar -What 'manifest temporary' | Out-Null
    Assert-Grammar -Value $ArtifactSha -Pattern $script:Sha256Grammar -What 'artifact hash' | Out-Null
    Assert-Grammar -Value $ManifestSha -Pattern $script:Sha256Grammar -What 'manifest hash' | Out-Null
    Assert-Grammar -Value $CandidateVersion -Pattern $script:VersionGrammar -What 'version' | Out-Null
    Assert-ManifestCas -Snapshot $ManifestCas | Out-Null
    if ($ArtifactSize -lt 0 -or $ArtifactSize -gt $script:MaxArtifactBytes -or $ManifestSize -le 0) {
        throw 'a publish transaction needs an artifact within the client ceiling and a manifest with bytes in it'
    }

    $template = @'
set -eu
web='@WEB@'
dir='@DIR@'
lock='@LOCK@'
artifact='@ARTIFACT@'
artifact_tmp='@ATMP@'
manifest='@MANIFEST@'
manifest_tmp='@MTMP@'
expected_cas='@CAS@'
artifact_sha='@ASHA@'
artifact_size='@ASIZE@'
manifest_sha='@MSHA@'
manifest_size='@MSIZE@'
mode='@MODE@'
candidate_version='@CVER@'

digest() { shasum -a 256 "$1" | cut -d' ' -f1; }
bytes() { wc -c < "$1" | tr -d ' '; }
snapshot() {
    if [ -f "$manifest" ]; then
        printf '%s %s' "$(digest "$manifest")" "$(bytes "$manifest")"
    elif [ -e "$manifest" ]; then
        printf 'unusable'
    else
        printf 'absent'
    fi
}
ver_order() {
    awk -v c="$1" -v p="$2" '
function split_ver(v, out,    i, num) {
    out[1] = 0
    out[2] = 0
    out[3] = 0
    out[4] = ""
    i = index(v, "-")
    if (i) {
        out[4] = substr(v, i + 1)
        v = substr(v, 1, i - 1)
    }
    split(v, num, ".")
    out[1] = num[1] + 0
    out[2] = num[2] + 0
    out[3] = num[3] + 0
}
BEGIN {
    if (c == p) { print "eq"; exit }
    split_ver(c, C)
    split_ver(p, P)
    for (i = 1; i <= 3; i++) {
        if (C[i] < P[i]) { print "lt"; exit }
        if (C[i] > P[i]) { print "gt"; exit }
    }
    cpre = C[4]
    ppre = P[4]
    if (cpre == "" && ppre == "") { print "eq"; exit }
    if (cpre == "") { print "gt"; exit }
    if (ppre == "") { print "lt"; exit }
    cn = split(cpre, ca, ".")
    pn = split(ppre, pa, ".")
    k = cn
    if (pn < k) k = pn
    for (i = 1; i <= k; i++) {
        cnum = (ca[i] ~ /^[0-9]+$/)
        pnum = (pa[i] ~ /^[0-9]+$/)
        if (cnum && pnum) {
            if (ca[i] + 0 < pa[i] + 0) { print "lt"; exit }
            if (ca[i] + 0 > pa[i] + 0) { print "gt"; exit }
        } else if (cnum && !pnum) { print "lt"; exit }
        else if (!cnum && pnum) { print "gt"; exit }
        else {
            if (ca[i] < pa[i]) { print "lt"; exit }
            if (ca[i] > pa[i]) { print "gt"; exit }
        }
    }
    if (cn < pn) { print "lt"; exit }
    if (cn > pn) { print "gt"; exit }
    print "eq"
}'
}

sudo -n mkdir "$lock" 2>/dev/null || {
    stale=$(sudo -n find "$lock" -prune -mmin +@STALE@ 2>/dev/null || true)
    if [ -n "$stale" ]; then
        sudo -n rmdir "$lock" 2>/dev/null || true
        sudo -n mkdir "$lock" 2>/dev/null || {
            printf 'another publish holds %s\n' "$lock" >&2
            exit 11
        }
    else
        printf 'another publish holds %s\n' "$lock" >&2
        exit 11
    fi
}
release() {
    release_status=$?
    sudo -n rm -f "$artifact_tmp" "$manifest_tmp" 2>/dev/null || true
    sudo -n rmdir "$lock" 2>/dev/null || true
    exit $release_status
}
trap release EXIT HUP INT TERM

observed="$(snapshot)"
if [ "$observed" != "$expected_cas" ]; then
    printf 'the published manifest changed under this publish (expected [%s], found [%s])\n' "$expected_cas" "$observed" >&2
    exit 12
fi

if [ -f "$manifest" ]; then
    published_live=$(awk -F '"' '$2 == "version" { print $4; exit }' "$manifest")
    [ -n "$published_live" ] || { printf 'the published manifest has no version\n' >&2; exit 28; }
    order=$(ver_order "$candidate_version" "$published_live")
    if [ "$order" = lt ]; then
        printf 'the candidate %s is older than the published %s\n' "$candidate_version" "$published_live" >&2
        exit 28
    fi
fi

[ -f "$artifact_tmp" ] || { printf 'the destination-local artifact temporary is gone\n' >&2; exit 13; }
[ "$(digest "$artifact_tmp")" = "$artifact_sha" ] || { printf 'the destination-local artifact temporary has other bytes\n' >&2; exit 14; }
[ "$(bytes "$artifact_tmp")" = "$artifact_size" ] || { printf 'the destination-local artifact temporary has another size\n' >&2; exit 15; }
[ -f "$manifest_tmp" ] || { printf 'the destination-local manifest temporary is gone\n' >&2; exit 16; }
[ "$(digest "$manifest_tmp")" = "$manifest_sha" ] || { printf 'the destination-local manifest temporary has other bytes\n' >&2; exit 17; }
[ "$(bytes "$manifest_tmp")" = "$manifest_size" ] || { printf 'the destination-local manifest temporary has another size\n' >&2; exit 18; }

if [ -e "$artifact" ]; then
    [ "$mode" = identical ] || { printf 'the release appeared under this publish\n' >&2; exit 19; }
    [ -f "$artifact" ] || { printf 'the published release is not a regular file\n' >&2; exit 20; }
    [ "$(digest "$artifact")" = "$artifact_sha" ] || { printf 'the published release has different bytes; a published version is immutable\n' >&2; exit 21; }
    [ "$(bytes "$artifact")" = "$artifact_size" ] || { printf 'the published release has another size\n' >&2; exit 22; }
    sudo -n rm -f "$artifact_tmp"
else
    [ "$mode" = new ] || { printf 'the release vanished under this publish\n' >&2; exit 23; }
    sudo -n mv "$artifact_tmp" "$artifact"
    sudo -n chmod 644 "$artifact"
fi

[ "$(digest "$artifact")" = "$artifact_sha" ] || { printf 'the final release does not carry the expected bytes\n' >&2; exit 24; }
[ "$(bytes "$artifact")" = "$artifact_size" ] || { printf 'the final release does not carry the expected size\n' >&2; exit 25; }

sudo -n mv "$manifest_tmp" "$manifest"
sudo -n chmod 644 "$manifest"
[ "$(digest "$manifest")" = "$manifest_sha" ] || { printf 'the published manifest does not carry the expected bytes\n' >&2; exit 26; }
[ "$(bytes "$manifest")" = "$manifest_size" ] || { printf 'the published manifest does not carry the expected size\n' >&2; exit 27; }

printf 'published %s %s\n' "$artifact_sha" "$manifest_sha"
'@
    $script = $template.
        Replace('@WEB@', $WebRoot).
        Replace('@DIR@', $RemoteDir).
        Replace('@LOCK@', "$WebRoot/.aitierlist-publish.lock").
        Replace('@ARTIFACT@', "$RemoteDir/$FileName").
        Replace('@ATMP@', $ArtifactTemp).
        Replace('@MANIFEST@', "$WebRoot/update.json").
        Replace('@MTMP@', $ManifestTemp).
        Replace('@CAS@', $ManifestCas).
        Replace('@ASHA@', $ArtifactSha).
        Replace('@ASIZE@', "$ArtifactSize").
        Replace('@MSHA@', $ManifestSha).
        Replace('@MSIZE@', "$ManifestSize").
        Replace('@MODE@', $Mode).
        Replace('@CVER@', $CandidateVersion).
        Replace('@STALE@', "$script:PublishLockStaleMinutes")
    return (Assert-NoPlaceholders -Script $script)
}

# The names this run owns in the destination directories. Hidden, GUID-named,
# and never a name anything serves.
function New-DestinationTempName {
    param(
        [Parameter(Mandatory)][string] $Directory,
        [Parameter(Mandatory)][string] $StageId,
        [Parameter(Mandatory)][string] $Leaf
    )

    Assert-Grammar -Value $Directory -Pattern $script:WebRootGrammar -What 'destination directory' | Out-Null
    Assert-Grammar -Value $StageId -Pattern $script:StageIdGrammar -What 'stage id' | Out-Null
    Assert-Grammar -Value $Leaf -Pattern '^[A-Za-z0-9._-]+$' -What 'temporary leaf' | Out-Null
    return "$Directory/.aitierlist-tmp-$StageId-$Leaf"
}

# The cleanup for the destination-local temporaries, for the paths that fail
# before the transaction ever runs.
function New-DestinationTempCleanupScript {
    param(
        [Parameter(Mandatory)][string] $ArtifactTemp,
        [Parameter(Mandatory)][string] $ManifestTemp
    )

    $template = @'
sudo -n rm -f '@ATMP@' '@MTMP@'
'@
    return (Assert-NoPlaceholders -Script ($template.
        Replace('@ATMP@', $ArtifactTemp).
        Replace('@MTMP@', $ManifestTemp)))
}

# Two lines of "<sha256> <size>", one per staged object, parsed exactly.
function Assert-HashSizeLine {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Line,
        [Parameter(Mandatory)][string] $Sha256,
        [Parameter(Mandatory)][long] $Size,
        [Parameter(Mandatory)][string] $What
    )

    # An explicit Match object rather than the $Matches automatic variable: what
    # $Matches holds after a -cnotmatch is a rule nobody should have to recall.
    $parsed = [regex]::Match($Line, '^([0-9a-f]{64}) ([0-9]{1,19})$')
    if (-not $parsed.Success) {
        throw "$What did not answer with a hash and a size: '$Line'"
    }
    $observedSha = $parsed.Groups[1].Value
    $observedSize = $parsed.Groups[2].Value
    if ($observedSha -cne $Sha256) {
        throw "$What hashes $observedSha, expected $Sha256"
    }
    if ($observedSize -cne "$Size") {
        throw "$What is $observedSize bytes, expected $Size"
    }
    return $true
}


function New-PackedArtifact {
    param(
        [Parameter(Mandatory)][System.IO.Stream] $Source,
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Upx,
        [Parameter(Mandatory)][string] $Root
    )

    $settings = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'aitierlist\settings.json'
    $scratch = Join-Path ([IO.Path]::GetTempPath()) ('aitierlist-smoke-' + [Guid]::NewGuid().ToString('N') + '.json')
    $cache = Join-Path $Root 'assets\aa-snapshot.json'
    $packed = $null
    try {
        Copy-Item -LiteralPath $settings -Destination $scratch
        foreach ($lzma in @($true, $false)) {
            $null = $Source.Seek(0, [IO.SeekOrigin]::Begin)
            $copy = [IO.File]::Open($Path, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::None)
            try { $Source.CopyTo($copy) }
            finally { $copy.Dispose() }
            $arguments = @('--best')
            if ($lzma) { $arguments += '--lzma' }
            $arguments += $Path
            $compression = Invoke-BoundedProcess -FilePath $Upx -ArgumentList $arguments -What 'packing the release executable'
            Write-Host $compression.StandardOutput.Trim()
            $packed = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
            try {
                Write-Host "  command  : & '$Path' --dump-table '--settings=$scratch' '--cache=$cache'"
                $smoke = Invoke-BoundedProcess -FilePath $Path -ArgumentList @('--dump-table', "--settings=$scratch", "--cache=$cache") `
                    -What 'the packed executable smoke test' -TimeoutSeconds 300 -MaxOutputBytes 64MB
            }
            catch {
                $packed.Dispose()
                $packed = $null
                if (-not $lzma) { throw }
                Write-Warning "The --best --lzma smoke test failed: $($_.Exception.Message). Retrying with --best without --lzma."
                continue
            }
            Write-Host "  smoke    : exit $($smoke.ExitCode), UPX $(if ($lzma) { '--best --lzma' } else { '--best' })" -ForegroundColor Green
            return $packed
        }
        throw 'the packed executable did not pass its smoke test'
    }
    catch {
        if ($null -ne $packed) { $packed.Dispose() }
        throw
    }
    finally {
        if (Test-Path -LiteralPath $scratch) { Remove-Item -LiteralPath $scratch -Force }
    }
}

# One remote command, through the shared bounded and noninteractive machinery.
# No `-o` of its own, no argument string, no shell: Invoke-RemoteCommand adds
# the control arguments - which include `-n`, because a control command's
# standard input is nothing at all - and throws on any non-zero exit.
function Invoke-Remote {
    param(
        [Parameter(Mandatory)][string] $Script,
        [string] $What = 'a remote command',
        [int] $TimeoutSeconds = 0
    )
    return Invoke-RemoteCommand -Ssh $script:ssh -RemoteHost $RemoteHost -Command $Script `
        -What $What -TimeoutSeconds $TimeoutSeconds
}

# ------------------------------------------------------------------ grammars

Assert-Grammar -Value $RemoteHost -Pattern (Get-HostGrammar) -What 'remote host' | Out-Null
Assert-Grammar -Value $WebRoot -Pattern (Get-WebRootGrammar) -What 'web root' | Out-Null

# Cargo.toml is the only source of the version. `-Version` is now an assertion
# rather than a choice: it may name the version being built, or nothing at all.
$line = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $line) { throw 'could not read version from Cargo.toml' }
$manifestVersion = $line.Matches[0].Groups[1].Value
$Version = Assert-RequestedVersion -Requested $Version -Manifest $manifestVersion
Assert-Grammar -Value $Version -Pattern (Get-VersionGrammar) -What 'version' | Out-Null
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "no release build at $exe - run: cargo build --release" }

$script:ssh = Resolve-SystemTool 'ssh.exe'
# No scp. Every byte this script uploads travels through the ssh standard input
# of an object it already holds open - see Send-StreamToRemoteFile for why a
# pathname-based copy is not equivalent.
$script:curl = Resolve-SystemTool 'curl.exe'

$fileName  = "aitierlist-$Version-portable-x64.exe"
$remoteDir = "$WebRoot/releases/$Version"
Assert-Grammar -Value $fileName -Pattern (Get-ArtifactNameGrammar) -What 'artifact name' | Out-Null
Assert-Grammar -Value $remoteDir -Pattern (Get-WebRootGrammar) -What 'release directory' | Out-Null

# ------------------------------------------------- one stable artifact, held

# The unpacked build is held against writes and deletion through inspection and
# copying. The packed copy is held through its smoke test, hash, size and upload.
$unpacked = [System.IO.File]::Open(
    $exe,
    [System.IO.FileMode]::Open,
    [System.IO.FileAccess]::Read,
    [System.IO.FileShare]::Read)
$artifact = $unpacked
try {
    Write-Host "AITIERLIST $Version" -ForegroundColor Cyan

    # The binary must actually have been built from this version, or the manifest
    # describes bytes that were never built from this tree. This is not paranoia: a
    # `cargo build` that fails because a running instance holds the .exe leaves the
    # PREVIOUS version sitting in target\release, and publishing that produces a
    # release which reports the old version after installing - so every client
    # re-offers the same update forever.
    #
    # Scanned rather than executed: AITIERLIST is a GUI-subsystem app, so running it just
    # opens a window (and leaves it open, locking the .exe for the next build).
    #
    # Read from the held handle, and matched as *exactly one* complete stamp.
    # `Contains` accepted a superset: the binary for 0.5.90 contains the string
    # "AITIERLIST_VERSION=0.5.9", and "AITIERLIST_VERSION=0.5.9.1" contains it too, so both
    # published happily as 0.5.9. Every stamp in the image is enumerated, they
    # must all be one value, and that value must equal this version.
    $reader = [System.IO.StreamReader]::new($unpacked, [System.Text.Encoding]::Latin1, $false, 1MB, $true)
    $bytes = $reader.ReadToEnd()
    $reader.Dispose()
    Assert-VersionStamp -Image $bytes -Version $Version -Path $exe | Out-Null
    $head = if ($AllowUnbound) {
        try { Get-HeadIdentity -Root $root }
        catch { [pscustomobject]@{ Commit = ('0' * 40); Tree = ('0' * 40) } }
    } else {
        Get-HeadIdentity -Root $root
    }
    Assert-HeadBinding -Image $bytes -Commit $head.Commit -Tree $head.Tree -Path $exe -AllowUnbound:$AllowUnbound | Out-Null
    $bytes = $null
    Write-Host "  stamp    : $Version confirmed in binary" -ForegroundColor Green
    Write-Host "  built    : $($head.Commit) tree $($head.Tree)" -ForegroundColor Green

    # Inspect the unpacked build's PE headers, load flags and imports before
    # compression changes their on-disk representation. Copy this held object.
    $image = Assert-ReleaseImage -Stream $unpacked -Path $exe
    Write-Host "  image    : $($image.Magic) $($image.Machine), DependentLoadFlags 0x$('{0:x4}' -f $image.DependentLoadFlags)" -ForegroundColor Green
    Write-Host "  imports  : $($image.Imports -join ', ')"
    $null = $artifact.Seek(0, [System.IO.SeekOrigin]::Begin)

    if ($WhatIf) {
        Write-Host "`n(WhatIf) would pack $exe to $packedExe with UPX --best --lzma, smoke-test it, then publish its hash and size. Nothing was changed." -ForegroundColor Yellow
        return
    }

    $artifact = New-PackedArtifact -Source $unpacked -Path $packedExe -Upx $upx -Root $root
    $size = $artifact.Length
    Assert-ArtifactSize -Size $size -What 'the packed release artifact' | Out-Null
    $sha = (Get-FileHash -InputStream $artifact -Algorithm SHA256).Hash.ToLower()
    $null = $artifact.Seek(0, [System.IO.SeekOrigin]::Begin)
    Write-Host ("  artifact : {0}  ({1} unpacked bytes, {2} packed bytes)" -f $fileName, $unpacked.Length, $size)
    Write-Host "  sha256   : $sha"
    Write-Host "  target   : ${RemoteHost}:$remoteDir"

    # The manifest, as bytes, before anything is uploaded. These exact bytes are
    # what goes up, what is hashed here, and what every check from the stage to
    # the published file compares against - there is no second serialization
    # anywhere, and no temporary file for anything to reopen by name.
    $expectedManifest = New-ReleaseManifest -Version $Version -FileName $fileName -Sha256 $sha -Size $size
    $manifestDocument = New-ReleaseManifestBytes -Manifest $expectedManifest
    Write-Host "  manifest : $($manifestDocument.Size) bytes, sha256 $($manifestDocument.Sha256)"

    # A unique staging name per run, local and remote. A fixed `$env:TEMP\aitierlist-update.json`
    # and a fixed `~/aitierlist-stage/<file>` are both names something else may already
    # occupy - another publish, or anything at all - and writing to them truncates
    # whatever is there.
    $stageId = [System.Guid]::NewGuid().ToString('N')
    $stageLeaf = ".aitierlist-publish-$stageId"
    $artifactTemp = New-DestinationTempName -Directory $remoteDir -StageId $stageId -Leaf $fileName
    $manifestTemp = New-DestinationTempName -Directory $WebRoot -StageId $stageId -Leaf 'update.json'

    # Advisory only. It decides which transaction to attempt; the server
    # rechecks the same facts under the publish lock, because anything observed
    # before the lock can change before the rename.
    $exists = Invoke-Remote -Script "test -f '$remoteDir/$fileName' && echo yes || echo no" -What 'the release existence check'
    $existingSha = ''
    if ($exists -eq 'yes') {
        $existingSha = Invoke-Remote -Script "shasum -a 256 '$remoteDir/$fileName' | cut -d' ' -f1" -What 'the published hash'
    }
    $republish = Assert-Republishable -Exists $exists -RemoteSha $existingSha -LocalSha $sha -Version $Version -Force:$Force
    if ($republish -ceq 'identical') {
        Write-Host "  note     : $Version is already published with these exact bytes; the release is left untouched and only the manifest is rewritten" -ForegroundColor Yellow
    }

    # The compare-and-set snapshot: exactly what is published right now, or an
    # explicit absent sentinel. The transaction refuses to run if this changed,
    # so two publishers racing produce one publication and one clean abort
    # rather than one publication rolled back over by the other.
    $manifestCas = Assert-ManifestCas -Snapshot (
        Invoke-Remote -Script (New-ManifestCasScript -WebRoot $WebRoot) -What 'the published manifest snapshot')
    $publishedVersion = Assert-PublishedVersion -Value (
        Invoke-Remote -Script (New-PublishedVersionScript -WebRoot $WebRoot) -What 'the published version')
    Assert-ReleaseMonotonic -Published $publishedVersion -Candidate $Version | Out-Null
    Write-Host "  live     : the published manifest is [$manifestCas]"
    Write-Host "  channel  : $publishedVersion"

    Write-Host "`nuploading..." -ForegroundColor Cyan
    # One absolute remote path, created exclusively and validated, used by every
    # command that follows.
    #
    # Absolute, never a tilde. Every remote command single-quotes this path, and
    # a tilde inside single quotes is not expanded by any POSIX shell: `mkdir -p
    # '~/.aitierlist-publish-<guid>'` makes a directory literally named `~` with that
    # child inside it, in whatever directory the ssh session happened to land in.
    # The absolute path the remote shell resolved is printed back and validated
    # before it is used for anything.
    #
    # `mkdir` without `-p`, so an existing name is an error rather than a
    # directory this run adopts, and `-m 700` so nothing else can write into the
    # artifact between the upload and the install.
    $makeStage = 'd="$HOME/' + $stageLeaf + '" && mkdir -m 700 "$d" && printf ''%s\n'' "$d"'
    $remoteStage = Assert-RemoteStagePath `
        -Path (Invoke-Remote -Script $makeStage -What 'creating the remote staging directory') `
        -Leaf $stageLeaf
    try {
        # The verified object itself, streamed. `umask 077` so the staged file
        # is not readable by anyone else between the write and the install, and
        # `exec cat` so the shell is replaced rather than left waiting on one.
        # The stage path is single-quoted, which is why it must not begin with a
        # tilde: a single-quoted ~ is a literal directory name.
        $null = $artifact.Seek(0, [System.IO.SeekOrigin]::Begin)
        Send-StreamToRemoteFile -Ssh $script:ssh -RemoteHost $RemoteHost `
            -RemoteCommand "umask 077 && exec cat > '$remoteStage/$fileName'" `
            -InputStream $artifact -What 'the artifact upload' | Out-Null

        # Verify on the server before it is reachable by anyone: catches a truncated or
        # corrupted transfer while the file is still outside the web root.
        $stagedArtifact = Invoke-Remote -What 'the staged artifact check' -Script (
            "printf '%s %s\n' " +
            "`"`$(shasum -a 256 '$remoteStage/$fileName' | cut -d' ' -f1)`" " +
            "`"`$(wc -c < '$remoteStage/$fileName' | tr -d ' ')`"")
        Assert-HashSizeLine -Line $stagedArtifact -Sha256 $sha -Size $size -What 'the staged artifact' | Out-Null
        Write-Host "  staged artifact verified on server" -ForegroundColor Green

        # The manifest's own bytes, up the same way and checked the same way.
        # They are a MemoryStream over the array that was hashed, so what is
        # verified on the server is the array this process holds.
        $manifestStream = [System.IO.MemoryStream]::new($manifestDocument.Bytes, $false)
        try {
            Send-StreamToRemoteFile -Ssh $script:ssh -RemoteHost $RemoteHost `
                -RemoteCommand "umask 077 && exec cat > '$remoteStage/update.json'" `
                -InputStream $manifestStream -What 'the manifest upload' | Out-Null
        }
        finally {
            $manifestStream.Dispose()
        }
        $stagedManifest = Invoke-Remote -What 'the staged manifest check' -Script (
            "printf '%s %s\n' " +
            "`"`$(shasum -a 256 '$remoteStage/update.json' | cut -d' ' -f1)`" " +
            "`"`$(wc -c < '$remoteStage/update.json' | tr -d ' ')`"")
        Assert-HashSizeLine -Line $stagedManifest -Sha256 $manifestDocument.Sha256 `
            -Size $manifestDocument.Size -What 'the staged manifest' | Out-Null
        Write-Host "  staged manifest verified on server" -ForegroundColor Green

        try {
            # Into the final directories, under names nothing serves. A rename
            # from here is same-directory and therefore atomic; a `mv` out of
            # $HOME can cross a filesystem, and a cross-device move is a copy
            # under the final name.
            $temps = @((Invoke-Remote -What 'copying into the destination directories' -Script (
                New-DestinationTempScript -WebRoot $WebRoot -RemoteDir $remoteDir -StagePath $remoteStage `
                    -FileName $fileName -ArtifactTemp $artifactTemp -ManifestTemp $manifestTemp)) -split "`r?`n" |
                ForEach-Object { $_.Trim() } | Where-Object { $_ })
            if ($temps.Count -ne 2) {
                throw "the destination-local copy answered with $($temps.Count) lines, expected two"
            }
            Assert-HashSizeLine -Line $temps[0] -Sha256 $sha -Size $size `
                -What 'the destination-local artifact temporary' | Out-Null
            Assert-HashSizeLine -Line $temps[1] -Sha256 $manifestDocument.Sha256 `
                -Size $manifestDocument.Size -What 'the destination-local manifest temporary' | Out-Null
            Write-Host "  destination-local temporaries verified" -ForegroundColor Green

            # One lock, one compare-and-set, one atomic rename each, manifest
            # last. Everything that can change the published tree is in here.
            $transacted = Invoke-Remote -What 'the publish transaction' -TimeoutSeconds 300 -Script (
                New-PublishTransactionScript -WebRoot $WebRoot -RemoteDir $remoteDir -FileName $fileName `
                    -ArtifactTemp $artifactTemp -ManifestTemp $manifestTemp -ManifestCas $manifestCas `
                    -ArtifactSha $sha -ArtifactSize $size -ManifestSha $manifestDocument.Sha256 `
                    -ManifestSize $manifestDocument.Size -Mode $republish -CandidateVersion $Version)
            if ($transacted -cne "published $sha $($manifestDocument.Sha256)") {
                throw "the publish transaction did not confirm this release: '$transacted'"
            }
            Write-Host "  published artifact and manifest verified in place" -ForegroundColor Green
        }
        finally {
            # This run's own destination-local temporaries, by name. The
            # transaction removes them itself on every path it reaches; this is
            # for the paths that never got that far.
            try {
                Invoke-Remote -What 'removing this run destination temporaries' -Script (
                    New-DestinationTempCleanupScript -ArtifactTemp $artifactTemp -ManifestTemp $manifestTemp) | Out-Null
            }
            catch {
                Write-Warning "removing this run's destination-local temporaries failed: $($_.Exception.Message)"
                Write-Warning "leftover temporaries on ${RemoteHost}: $artifactTemp, $manifestTemp"
            }
        }
    }
    finally {
        # Only this run's own staging directory, named by its own GUID. It
        # cannot throw from here without swallowing whatever brought us into the
        # `finally`, so a failure is reported loudly instead: a staging
        # directory left on the server is a thing somebody has to know about.
        try {
            Invoke-Remote -Script "rm -rf '$remoteStage'" -What 'removing the remote staging directory' | Out-Null
        }
        catch {
            Write-Warning "removing the remote staging directory failed: $($_.Exception.Message)"
            Write-Warning "leftover staging directory on ${RemoteHost}: $remoteStage"
        }
    }

    Write-Host "`nverifying live endpoint..." -ForegroundColor Cyan
    # The raw body, handed to the validator unparsed. `ConvertFrom-Json` folds
    # property case, hides duplicate keys and unwraps a one-element top-level
    # array into the object inside it, so anything it produced would be a
    # different document from the one the endpoint actually served.
    $live = (Invoke-BoundedProcess -FilePath $script:curl -What 'fetching the live manifest' `
        -TimeoutSeconds 60 -ArgumentList @(
            '-fsS', '--max-time', '20',
            'https://files.blockitall.us/aitierlist/update.json')).StandardOutput

    Assert-LiveManifest -Json $live -Expected $expectedManifest | Out-Null
    Write-Host "  manifest : live, every field matches" -ForegroundColor Green

    # The served object, not a one-byte Range. HTTP 200/206 on byte zero is how
    # a stale, truncated, corrupt or misrouted body still printed "published."
    $probe = Join-Path ([System.IO.Path]::GetTempPath()) ('.aitierlist-live-' + [System.Guid]::NewGuid().ToString('N'))
    try {
        $code = (Invoke-BoundedProcess -FilePath $script:curl -What 'fetching the live artifact' `
            -TimeoutSeconds 900 -ArgumentList @(
                '-sS', '-o', $probe, '-w', '%{http_code}', '--max-time', '600',
                '--max-filesize', "$size",
                "https://files.blockitall.us/aitierlist/releases/$Version/$fileName")).StandardOutput.Trim()
        $served = [System.IO.File]::Open(
            $probe,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            [System.IO.FileShare]::Read)
        try {
            $servedSize = $served.Length
            $servedSha = (Get-FileHash -InputStream $served -Algorithm SHA256).Hash.ToLower()
        }
        finally {
            $served.Dispose()
        }
        Assert-LiveArtifact -Code $code -Size $servedSize -Sha256 $servedSha `
            -ExpectedSize $size -ExpectedSha256 $sha | Out-Null
        Write-Host "  artifact : HTTP $code, $servedSize bytes, sha256 $servedSha" -ForegroundColor Green
    }
    finally {
        if (Test-Path -LiteralPath $probe -PathType Leaf) {
            Remove-Item -LiteralPath $probe -Force -ErrorAction SilentlyContinue
        }
    }
    Write-Host "`npublished." -ForegroundColor Green
}
finally {
    $artifact.Dispose()
    if ($unpacked -ne $artifact) { $unpacked.Dispose() }
}

