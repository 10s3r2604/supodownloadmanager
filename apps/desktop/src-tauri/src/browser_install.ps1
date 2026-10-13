# Invoked only after the user selects a browser and starts setup.
# Use that browser's normal, enabled UI controls. No profile files, policies,
# remote-debugging ports or browser security settings are bypassed.
$ErrorActionPreference = 'Stop'
try {
    [Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $script:stage = 'provider'
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    # A compiled caller lets WPF resolve its built-in provider assembly without
    # encountering PowerShell's dynamic stack frames.
    Add-Type -ReferencedAssemblies ([Windows.Automation.AutomationElement].Assembly.Location) -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SdmBrowserWindow {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr key, IntPtr flags);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window, uint kind);
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    public static void InitializeAutomation() {
        var proxy = typeof(System.Windows.Automation.AutomationElement).Assembly.GetName();
        proxy.Name = "UIAutomationClientsideProviders";
        System.Windows.Automation.ClientSettings.RegisterClientSideProviderAssembly(proxy);
    }
    public static bool IsOwnedBy(IntPtr window, IntPtr owner) {
        for (int i = 0; i < 8; i++) {
            window = GetWindow(window, 4);
            if (window == IntPtr.Zero) return false;
            if (window == owner) return true;
        }
        return false;
    }
}
'@
    [SdmBrowserWindow]::InitializeAutomation()
    $script:stage = 'locate-window'
    $script:navigated = $false
    function Wait-For($action) {
        while ($clock.Elapsed.TotalSeconds -lt 22) {
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
                $nativeAddress = $null
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
                    if (-not $edit.Current.HasKeyboardFocus -and $null -ne $address -and $address.Trim().TrimEnd('/') -eq $request.page.TrimEnd('/')) {
                        return @{root=$root; pid=$process.Id}
                    }
                    if ($edit.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and
                        $edit.Current.Name -in @('Address and search bar', 'Thanh địa chỉ và tìm kiếm')) {
                        $nativeAddress = $edit
                    }
                }
                if ($request.engine -eq 'chromium' -and -not $script:navigated -and $null -ne $nativeAddress) {
                    # A fresh browser can ignore a management URL passed at
                    # startup. Navigate once through its native address field.
                    # Send Enter to this verified browser window only, after
                    # checking the field's focus; never use global keystrokes.
                    $script:stage = 'navigate'
                    $nativeAddress.SetFocus()
                    Start-Sleep -Milliseconds 50
                    $focused = [Windows.Automation.AutomationElement]::FocusedElement
                    $handle = [IntPtr]::new($root.Current.NativeWindowHandle)
                    if ([SdmBrowserWindow]::GetForegroundWindow() -eq $handle -and
                        $null -ne $focused -and
                        ($focused.GetRuntimeId() -join ',') -eq ($nativeAddress.GetRuntimeId() -join ',')) {
                        $value = $nativeAddress.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
                        if (-not $value.Current.IsReadOnly) {
                            $value.SetValue($request.page)
                            [SdmBrowserWindow]::PostMessage($handle, 0x100, [IntPtr]::new(13), [IntPtr]::new(0x001C0001)) | Out-Null
                            [SdmBrowserWindow]::PostMessage($handle, 0x101, [IntPtr]::new(13), [IntPtr]::new(0xC01C0001)) | Out-Null
                            $script:navigated = $true
                        }
                    }
                }
            } catch { }
        return $null
    }
    if ($null -eq $window) { throw 'The extensions page is not available through accessibility.' }
    $root = $window.root
    $script:stage = 'page'
    $management = Wait-For {
        $documents = $root.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Document))
        foreach ($document in $documents) {
            if ($document.Current.IsOffscreen) { continue }
            $value = $null
            if ($document.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$value) -and
                $value.Current.Value.TrimEnd('/') -eq $request.page.TrimEnd('/')) { return $document }
        }
        return $null
    }
    if ($null -eq $management) { throw 'The management document is not ready.' }
    if ($request.updating) {
        # Reload an existing card only. Never load a second SDM copy to update.
        if ($request.engine -eq 'chromium') {
            $developer = Find-Control $management @('Developer mode', 'Turn on Developer mode', 'Turn off Developer mode', 'Chế độ dành cho nhà phát triển', 'Bật chế độ dành cho nhà phát triển', 'Tắt chế độ dành cho nhà phát triển') ([Windows.Automation.TogglePattern]::Pattern)
            if ($null -ne $developer) {
                $toggle = $developer.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
                if ($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) { $toggle.Toggle() }
            }
        }
        $script:stage = 'reload'
        $reload = Wait-For {
            $name = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Supo Download Manager')
            $brands = @($management.FindAll([Windows.Automation.TreeScope]::Descendants, $name))
            $matches = @()
            foreach ($brand in $brands) {
                $card = $brand
                for ($level = 0; $level -lt 4 -and $null -ne $card -and $card -ne $management; $level++) {
                    $control = Find-Control $card @('Reload', 'Reload extension', 'Tải lại', 'Tải lại tiện ích') ([Windows.Automation.InvokePattern]::Pattern)
                    if ($null -ne $control) { $matches += $control; break }
                    $card = [Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($card)
                }
            }
            $matches = @($matches | Sort-Object { $_.GetRuntimeId() -join ',' } -Unique)
            if ($matches.Count -eq 1) { return $matches[0] }
            return $null
        }
        if ($null -eq $reload) { throw 'Reload requires confirmation.' }
        $reload.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        $script:stage = 'verify'
        $updated = Wait-For {
            $name = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Supo Download Manager')
            $brand = $management.FindFirst([Windows.Automation.TreeScope]::Descendants, $name)
            $version = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $request.version)
            $card = $brand
            for ($level = 0; $level -lt 4 -and $null -ne $card -and $card -ne $management; $level++) {
                if ($null -ne $card.FindFirst([Windows.Automation.TreeScope]::Descendants, $version)) { return $true }
                $card = [Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($card)
            }
            return $null
        }
        if ($updated -eq $true) { [Console]::Out.Write('loaded'); exit 0 }
        throw 'The updated version has not been confirmed.'
    }
    if ($request.engine -eq 'chromium') {
        $script:stage = 'developer'
        $menu = $null
        $developerNames = @('Developer mode', 'Turn on Developer mode', 'Turn off Developer mode', 'Chế độ dành cho nhà phát triển', 'Bật chế độ dành cho nhà phát triển', 'Tắt chế độ dành cho nhà phát triển')
        $developer = Find-Control $management $developerNames ([Windows.Automation.TogglePattern]::Pattern)
        if ($null -eq $developer) {
            # Edge's narrow extensions page puts this switch in its sidebar.
            $menu = Find-Control $management @('Extensions menu', 'Menu tiện ích mở rộng') ([Windows.Automation.InvokePattern]::Pattern)
            if ($null -ne $menu) { $menu.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
            $developer = Wait-For { Find-Control $management $developerNames ([Windows.Automation.TogglePattern]::Pattern) }
        }
        if ($null -eq $developer) { throw 'Developer mode requires confirmation.' }
        $toggle = $developer.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
        if ($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) { $toggle.Toggle() }
        $enabled = Wait-For {
            if ($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::On) { return $true }
            return $null
        }
        if ($enabled -ne $true) { throw 'Developer mode was not enabled.' }
        # Return from Edge's sidebar to the list containing Load unpacked.
        if ($null -ne $menu) {
            $listNames = @('My extensions', 'Tiện ích mở rộng của tôi', 'Tiện ích của tôi')
            $list = Find-Control $management $listNames ([Windows.Automation.InvokePattern]::Pattern)
            if ($null -ne $list) { $list.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
            else {
                $list = Find-Control $management $listNames ([Windows.Automation.SelectionItemPattern]::Pattern)
                if ($null -ne $list) { $list.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select() }
            }
        }
        $script:stage = 'load'
        $load = Wait-For { Find-Control $management @('Load unpacked', 'Tải tiện ích đã giải nén', 'Tải phần mở rộng đã giải nén') ([Windows.Automation.InvokePattern]::Pattern) }
        $path = $request.directory
    } else {
        $script:stage = 'load'
        $load = Wait-For { Find-Control $management @('Load Temporary Add-on…', 'Load Temporary Add-on...', 'Tải tiện ích tạm thời…') ([Windows.Automation.InvokePattern]::Pattern) }
        $path = Join-Path $request.directory 'manifest.json'
    }
    if ($null -eq $load) { throw 'Loading requires confirmation.' }
    $load.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    $script:stage = 'picker'
    $dialogCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ClassNameProperty, '#32770')
    function Get-OwnedDialogs {
        # Windows can expose the picker as an owned child of the browser, or
        # through its shell host. Verify native ownership, not the UIA PID.
        $candidates = @([Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $dialogCondition)) +
            @($root.FindAll([Windows.Automation.TreeScope]::Descendants, $dialogCondition))
        return ,@($candidates | Where-Object {
            [SdmBrowserWindow]::IsOwnedBy([IntPtr]::new($_.Current.NativeWindowHandle), [IntPtr]::new($root.Current.NativeWindowHandle))
        } | Sort-Object { $_.GetRuntimeId() -join ',' } -Unique)
    }
    $dialog = Wait-For {
        $dialogs = Get-OwnedDialogs
        if ($dialogs.Count -eq 1) { return $dialogs[0] }
        return $null
    }
    if ($null -eq $dialog) { throw 'The browser file picker is not available.' }
    $field = Wait-For {
        # File pickers use 1148; folder pickers use the native 1152 edit.
        foreach ($id in @('1148', '1152')) {
            $candidate = $dialog.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $id))
            if ($null -ne $candidate) { return $candidate }
        }
        return $null
    }
    if ($null -eq $field) { throw 'The browser file picker requires confirmation.' }
    $provider = $null
    if (-not $field.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$provider)) {
        $field = $field.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit))
        if ($null -eq $field -or -not $field.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$provider)) { throw 'No writable file-picker field.' }
    }
    $provider.SetValue($path)
    $script:stage = 'confirm'
    $confirm = $dialog.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, '1'))
    if ($null -eq $confirm -or -not $confirm.Current.IsEnabled) { throw 'The browser requires confirmation.' }
    $confirm.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    $script:stage = 'verify'
    $loaded = Wait-For {
        $dialogs = Get-OwnedDialogs
        if ($dialogs.Count -ne 0) { return $null }
        $name = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Supo Download Manager')
        $brand = $management.FindFirst([Windows.Automation.TreeScope]::Descendants, $name)
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
[Console]::Out.Write("manual:$($script:stage)")
