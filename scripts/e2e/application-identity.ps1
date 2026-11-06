param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\..\target\release\brightwheel.exe')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$manifest = Join-Path $PSScriptRoot '..\..\Cargo.toml'
$metadataJson = & cargo metadata --manifest-path $manifest --no-deps --format-version 1 --locked
if ($LASTEXITCODE -ne 0) { throw 'Could not read the package version.' }
$package = ($metadataJson | ConvertFrom-Json).packages | Where-Object name -eq 'brightwheel'
if (-not $package) { throw 'The brightwheel package was not found.' }

$path = (Resolve-Path $Executable).Path
$info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($path)
$coreVersion = [Version](($package.version -split '[-+]', 2)[0])
$expected = @{
    FileDescription = 'BrightWheel'
    ProductName = 'BrightWheel'
    InternalName = 'brightwheel'
    OriginalFilename = 'brightwheel.exe'
    FileVersion = $package.version
    ProductVersion = $package.version
    FileMajorPart = $coreVersion.Major
    FileMinorPart = $coreVersion.Minor
    FileBuildPart = $coreVersion.Build
    FilePrivatePart = 0
    ProductMajorPart = $coreVersion.Major
    ProductMinorPart = $coreVersion.Minor
    ProductBuildPart = $coreVersion.Build
    ProductPrivatePart = 0
    IsPreRelease = (($package.version -split '\+', 2)[0]).Contains('-')
}
foreach ($field in $expected.Keys) {
    if ($info.$field -cne $expected[$field]) {
        throw "Invalid $field in ${path}: expected '$($expected[$field])', got '$($info.$field)'."
    }
}

Write-Host "PASS: Windows application identity is BrightWheel, version $($info.FileVersion), original filename $($info.OriginalFilename)"

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class BrightWheelIdentityNative {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibraryExW(string path, IntPtr reserved, uint flags);
    [DllImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool FreeLibrary(IntPtr module);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr LoadImageW(IntPtr module, IntPtr name, uint type, int width, int height, uint flags);
    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool DestroyIcon(IntPtr icon);
}
'@

# Inspect the built EXE's resources without running its code.
# LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE
$module = [BrightWheelIdentityNative]::LoadLibraryExW($path, [IntPtr]::Zero, 0x22)
if ($module -eq [IntPtr]::Zero) { throw [ComponentModel.Win32Exception]::new() }
try {
    $dotCenters = @(@(3, 15), @(6, 4), @(16, 1), @(26, 4), @(29, 15))
    foreach ($resourceId in @(1, 2)) {
        # IMAGE_ICON; omit LR_SHARED so the test owns the returned handle.
        $handle = [BrightWheelIdentityNative]::LoadImageW($module, [IntPtr]$resourceId, 1, 32, 32, 0)
        if ($handle -eq [IntPtr]::Zero) { throw [ComponentModel.Win32Exception]::new() }
        try {
            $icon = [Drawing.Icon]::FromHandle($handle)
            try {
                $bitmap = $icon.ToBitmap()
                try {
                    if ($bitmap.Width -ne 32 -or $bitmap.Height -ne 32) {
                        throw "Resource $resourceId did not load at the requested 32x32 size."
                    }
                    foreach ($point in $dotCenters) {
                        $pixel = $bitmap.GetPixel($point[0], $point[1])
                        # Accept both straight and premultiplied amber channels.
                        $isAmber = $pixel.A -ge 200 -and $pixel.R -ge 200 -and $pixel.G -ge 160 -and $pixel.B -le 100
                        if ($isAmber -ne ($resourceId -eq 1)) {
                            throw "Unexpected indicator in resource $resourceId at ($($point -join ',')): $pixel"
                        }
                    }
                } finally { $bitmap.Dispose() }
            } finally { $icon.Dispose() }
        } finally { [void][BrightWheelIdentityNative]::DestroyIcon($handle) }
    }
} finally { [void][BrightWheelIdentityNative]::FreeLibrary($module) }

Write-Host 'PASS: compiled application icon has all five amber dots; the dynamic tray bulb is a separate resource'
