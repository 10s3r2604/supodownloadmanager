# Invoked only after the user selects a browser and starts setup.
# Use that browser's normal, enabled UI controls. No profile files, policies,
# remote-debugging ports or browser security settings are bypassed.
$ErrorActionPreference = 'Stop'
try {
    [Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type 'using System; using System.Runtime.InteropServices; public static class SdmBrowserWindow { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); }'
    $clock = [Diagnostics.Stopwatch]::StartNew()
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
                if ($handle -eq [IntPtr]::Zero) { return $null }
                $root = [Windows.Automation.AutomationElement]::FromHandle($handle)
                $process = Get-Process -Id $root.Current.ProcessId
                if ($process.Path -ne $request.executable) { return $null }
                $edits = $root.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit))
                foreach ($edit in $edits) {
                    $provider = $null
                    if ($edit.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$provider) -and $provider.Current.Value.TrimEnd('/') -eq $request.page.TrimEnd('/')) {
                        return @{root=$root; pid=$process.Id}
                    }
                }
            } catch { }
        return $null
    }
    if ($null -eq $window) { throw 'The extensions page is not available through accessibility.' }
    $root = $window.root
    if ($request.engine -eq 'chromium') {
        $developer = Find-Control $root @('Developer mode', 'Chế độ dành cho nhà phát triển') ([Windows.Automation.TogglePattern]::Pattern)
        if ($null -eq $developer) { throw 'Developer mode requires confirmation.' }
        $toggle = $developer.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
        if ($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) { $toggle.Toggle() }
        $load = Wait-For { Find-Control $root @('Load unpacked', 'Tải tiện ích đã giải nén', 'Tải phần mở rộng đã giải nén') ([Windows.Automation.InvokePattern]::Pattern) }
        $path = $request.directory
    } else {
        $load = Wait-For { Find-Control $root @('Load Temporary Add-on…', 'Load Temporary Add-on...', 'Tải tiện ích tạm thời…') ([Windows.Automation.InvokePattern]::Pattern) }
        $path = Join-Path $request.directory 'manifest.json'
    }
    if ($null -eq $load) { throw 'Loading requires confirmation.' }
    $load.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
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
    $confirm = $dialog.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, '1'))
    if ($null -eq $confirm -or -not $confirm.Current.IsEnabled) { throw 'The browser requires confirmation.' }
    $confirm.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
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
[Console]::Out.Write('manual')
