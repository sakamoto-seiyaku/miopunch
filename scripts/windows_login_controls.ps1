param(
    [Parameter(Mandatory=$true)][Int64]$Window,
    [Parameter(Mandatory=$true)][ValidateSet('inspect','off','quit')][string]$Action
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$miopunchRoot = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Window)
if ($null -eq $miopunchRoot) { throw 'owned automation root unavailable' }
$miopunchElements = $miopunchRoot.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
$miopunchToggle = $null
$miopunchQuit = $null
$miopunchControls = @()
foreach ($miopunchElement in $miopunchElements) {
    $miopunchCurrent = $miopunchElement.Current
    if ($miopunchCurrent.ControlType -eq [Windows.Automation.ControlType]::CheckBox -or
        $miopunchCurrent.ControlType -eq [Windows.Automation.ControlType]::Button) {
        $miopunchControls += @{name=$miopunchCurrent.Name;type=$miopunchCurrent.ControlType.ProgrammaticName}
    }
    if ($miopunchCurrent.ControlType -eq [Windows.Automation.ControlType]::CheckBox -and
        $miopunchCurrent.Name.Trim() -eq 'Start silently at login') { $miopunchToggle = $miopunchElement }
    if ($miopunchCurrent.ControlType -eq [Windows.Automation.ControlType]::Button -and
        $miopunchCurrent.Name.Trim() -eq 'Quit node and host') { $miopunchQuit = $miopunchElement }
}
if ($null -eq $miopunchToggle -or $null -eq $miopunchQuit) {
    @{status='missing_controls';controls=$miopunchControls}|ConvertTo-Json -Depth 5 -Compress
    throw 'actual bundled controls unavailable'
}
$miopunchPattern = $miopunchToggle.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
$miopunchBefore = $miopunchPattern.Current.ToggleState.ToString()
if ($Action -eq 'off') {
    if ($miopunchBefore -ne 'On') { throw 'actual autostart control was not On' }
    $miopunchPattern.Toggle()
}
$miopunchAfter = $miopunchPattern.Current.ToggleState.ToString()
@{status='pass';action=$Action;before=$miopunchBefore;after=$miopunchAfter;controls=$miopunchControls;window=$Window}|ConvertTo-Json -Depth 5 -Compress
if ($Action -eq 'quit') {
    $miopunchInvoke = $miopunchQuit.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
    $miopunchInvoke.Invoke()
}
