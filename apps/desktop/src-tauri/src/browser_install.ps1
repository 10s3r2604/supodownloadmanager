# Invoked only after the user selects a browser and starts setup.
# Use that browser's normal, enabled UI controls. No profile files, policies,
# remote-debugging ports or browser security settings are bypassed.
$ErrorActionPreference = 'Stop'
try {
    [Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $clock = [Diagnostics.Stopwatch]::StartNew()
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type 'using System; using System.Runtime.InteropServices; public static class SdmBrowserWindow { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); }'
    $script:stage = 'locate-window'
    function Wait-For($action) {
        while ($clock.Elapsed.TotalSeconds -lt 18) {
            $value = & $action
            if ($null -ne $value) { return $value }
            Start-Sleep -Milliseconds 250
        }
        return $null
    }
    function Find-Control($root, $names, $pattern) {
        foreach ($name in $names) {
            $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $name)
            foreach ($element in $root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)) {
                if (-not $element.Current.IsEnabled -or $element.Current.IsOffscreen) { continue }
                $provider = $null
                if ($element.TryGetCurrentPattern($pattern, [ref]$provider)) { return $element }
            }
        }
        return $null
    }
    $window = Wait-For {
            try {
                $handle = [SdmBrowserWindow]::GetForegroundWindow()
                $root = $null
                if ($handle -ne [IntPtr]::Zero) {
                    $foreground = [Windows.Automation.AutomationElement]::FromHandle($handle)
                    $process = Get-Process -Id $foreground.Current.ProcessId
                    if ($process.Path -eq $request.executable) { $root = $foreground }
                }
                if ($null -eq $root) {
                    # A service/CI desktop may have no foreground window. A
                    # single browser window is unambiguous; multiple windows
                    # or profiles still require the user to choose/focus one.
                    $windows = @([Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, [Windows.Automation.Condition]::TrueCondition) | Where-Object {
                        try { (Get-Process -Id $_.Current.ProcessId).Path -eq $request.executable } catch { $false }
                    })
                    if ($windows.Count -ne 1) { return $null }
                    $root = $windows[0]
                    $process = Get-Process -Id $root.Current.ProcessId
                }
                $script:stage = 'locate-address'
                $addressTypes = [Windows.Automation.OrCondition]::new(
                    [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::IsValuePatternAvailableProperty, $true),
                    [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::IsTextPatternAvailableProperty, $true))
                $edits = $root.FindAll([Windows.Automation.TreeScope]::Descendants, $addressTypes)
                foreach ($edit in $edits) {
                    # Page inputs can impersonate a URL; only the browser's
                    # address control outside document content is trusted.
                    $parent = $edit
                    $inDocument = $false
                    while ($null -ne $parent -and $parent -ne $root) {
                        if ($parent.Current.ControlType -eq [Windows.Automation.ControlType]::Document) { $inDocument = $true; break }
                        $parent = [Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($parent)
                    }
                    if ($inDocument) { continue }
                    $script:stage = 'locate-url'
                    $provider = $null
                    $address = $null
                    if ($edit.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$provider)) { $address = $provider.Current.Value }
                    elseif ($edit.TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern, [ref]$provider)) { $address = $provider.DocumentRange.GetText(-1) }
                    if ($null -ne $address -and $address.Trim().TrimEnd('/') -eq $request.page.TrimEnd('/')) {
                        return @{root=$root; pid=$process.Id}
                    }
                }
            } catch { }
        return $null
    }
    if ($null -eq $window) { throw 'The extensions page is not available through accessibility.' }
    $root = $window.root
    if ($request.engine -eq 'chromium') {
        $stage = 'developer'
        $developer = Find-Control $root @('Developer mode', 'Chế độ dành cho nhà phát triển') ([Windows.Automation.TogglePattern]::Pattern)
        if ($null -eq $developer) { throw 'Developer mode requires confirmation.' }
        $toggle = $developer.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
        if ($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) { $toggle.Toggle() }
        $stage = 'load'
        $load = Wait-For { Find-Control $root @('Load unpacked', 'Tải tiện ích đã giải nén', 'Tải phần mở rộng đã giải nén') ([Windows.Automation.InvokePattern]::Pattern) }
        $path = $request.directory
    } else {
        $stage = 'load'
        $load = Wait-For { Find-Control $root @('Load Temporary Add-on…', 'Load Temporary Add-on...', 'Tải tiện ích tạm thời…') ([Windows.Automation.InvokePattern]::Pattern) }
        $path = Join-Path $request.directory 'manifest.json'
    }
    if ($null -eq $load) { throw 'Loading requires confirmation.' }
    $load.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    $stage = 'picker'
    $dialogCondition = [Windows.Automation.AndCondition]::new(
        [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$window.pid),
        [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ClassNameProperty, '#32770'))
    $dialog = Wait-For {
        $dialogs = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $dialogCondition)
        if ($dialogs.Count -eq 1) { return $dialogs[0] }
        return $null
    }
    if ($null -eq $dialog) { throw 'The browser file picker is not available.' }
    $field = $dialog.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, '1148'))
    if ($null -eq $field) { throw 'The browser file picker requires confirmation.' }
    $provider = $null
    if (-not $field.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$provider)) {
        $field = $field.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit))
        if ($null -eq $field -or -not $field.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$provider)) { throw 'No writable file-picker field.' }
    }
    $provider.SetValue($path)
    $stage = 'confirm'
    $confirm = $dialog.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, '1'))
    if ($null -eq $confirm -or -not $confirm.Current.IsEnabled) { throw 'The browser requires confirmation.' }
    $confirm.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    $stage = 'verify'
    $loaded = Wait-For {
        $dialogs = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $dialogCondition)
        if ($dialogs.Count -ne 0) { return $null }
        $name = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Supo Download Manager')
        $brand = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $name)
        if ($null -ne $brand) {
            $version = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $request.version)
            $card = $brand
            for ($level = 0; $level -lt 4 -and $null -ne $card; $level++) {
                if ($null -ne $card.FindFirst([Windows.Automation.TreeScope]::Descendants, $version)) { return $true }
                $card = [Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($card)
            }
        }
        return $null
    }
    if ($loaded -eq $true) { [Console]::Out.Write('loaded'); exit 0 }
} catch { }
[Console]::Out.Write("manual:$stage")
