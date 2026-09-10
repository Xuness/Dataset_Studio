param([string]$ExpectedPath, [string]$CaseName, [switch]$Remove)
# Windows PowerShell 5.1 provides the WinRT projection used by this optional test.
# Compare only the supplied fixture text; never print other clipboard contents.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$clipboardType = [Windows.ApplicationModel.DataTransfer.Clipboard, Windows.ApplicationModel.DataTransfer, ContentType=WindowsRuntime]
$resultType = [Windows.ApplicationModel.DataTransfer.ClipboardHistoryItemsResult, Windows.ApplicationModel.DataTransfer, ContentType=WindowsRuntime]
$asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetGenericArguments().Count -eq 1 -and
    $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
} | Select-Object -First 1
function Await-WinRT($Operation, [Type]$Type) {
    $task = $asTask.MakeGenericMethod($Type).Invoke($null,@($Operation))
    if (-not $task.Wait(10000)) { throw 'Clipboard query timed out.' }
    return $task.Result
}
function Normalize-Text([string]$Text) { $Text.Replace("`r`n", "`n") }
$expected = Get-Content -LiteralPath $ExpectedPath -Raw -Encoding UTF8
$result = Await-WinRT ($clipboardType::GetHistoryItemsAsync()) $resultType
$matches = @()
$removed = 0
foreach ($item in $result.Items) {
    if ($item.Content.Contains('Text')) {
        $itemText = Await-WinRT ($item.Content.GetTextAsync()) ([string])
        if ((Normalize-Text $itemText) -ceq (Normalize-Text $expected)) {
            $matches += [ordered]@{timestamp=$item.Timestamp.ToString('o');formats=@($item.Content.AvailableFormats)}
            if ($Remove -and $clipboardType::DeleteItemFromHistory($item)) { $removed++ }
        }
    }
}
$currentText = Get-Clipboard -Raw
[ordered]@{
    case=$CaseName
    capturedAt=(Get-Date -Format o)
    isHistoryEnabled=$clipboardType::IsHistoryEnabled()
    status=$result.Status.ToString()
    currentMatches=((Normalize-Text $currentText) -ceq (Normalize-Text $expected))
    historyMatches=$matches
    removed=$removed
} | ConvertTo-Json -Compress -Depth 6
