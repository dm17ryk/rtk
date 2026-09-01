param(
    [string]$Output = "src/cmds/powershell/manifest_snapshot.json"
)

# Offline and deterministic by design.  The snapshot is checked in so release
# builds never fetch module metadata from the network.  Runtime validation is
# performed by `cargo test powershell::manifest`.
$entries = @(
    [ordered]@{ canonical_name = 'Get-ChildItem'; aliases = @('gci','ls','dir'); module = 'Microsoft.PowerShell.Management'; kind = 'cmdlet'; hosts = @('Desktop51','Pwsh74','Pwsh75','Pwsh76'); behavior = 'table'; strategy = 'specialized:filesystem' },
    [ordered]@{ canonical_name = 'Get-Process'; aliases = @('gps','ps'); module = 'Microsoft.PowerShell.Management'; kind = 'cmdlet'; hosts = @('Desktop51','Pwsh74','Pwsh75','Pwsh76'); behavior = 'table'; strategy = 'specialized:process' },
    [ordered]@{ canonical_name = 'Get-Service'; aliases = @('gsv'); module = 'Microsoft.PowerShell.Management'; kind = 'cmdlet'; hosts = @('Desktop51','Pwsh74','Pwsh75','Pwsh76'); behavior = 'table'; strategy = 'specialized:service' },
    [ordered]@{ canonical_name = 'Get-Command'; aliases = @('gcm'); module = 'Microsoft.PowerShell.Core'; kind = 'cmdlet'; hosts = @('Desktop51','Pwsh74','Pwsh75','Pwsh76'); behavior = 'table'; strategy = 'specialized:discovery' },
    [ordered]@{ canonical_name = 'Get-Module'; aliases = @('gmo'); module = 'Microsoft.PowerShell.Core'; kind = 'cmdlet'; hosts = @('Desktop51','Pwsh74','Pwsh75','Pwsh76'); behavior = 'table'; strategy = 'specialized:discovery' },
    [ordered]@{ canonical_name = 'Get-CimInstance'; aliases = @('gcim'); module = 'CimCmdlets'; kind = 'cmdlet'; hosts = @('Pwsh74','Pwsh75','Pwsh76'); behavior = 'table'; strategy = 'specialized:cim' },
    [ordered]@{ canonical_name = 'Get-ADUser'; aliases = @(); module = 'ActiveDirectory'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:active-directory' },
    [ordered]@{ canonical_name = 'Get-VM'; aliases = @(); module = 'Hyper-V'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:hyper-v' },
    [ordered]@{ canonical_name = 'Get-MpComputerStatus'; aliases = @(); module = 'Defender'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:defender' },
    [ordered]@{ canonical_name = 'Get-BitLockerVolume'; aliases = @(); module = 'BitLocker'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:bitlocker' },
    [ordered]@{ canonical_name = 'Get-ScheduledTask'; aliases = @(); module = 'ScheduledTasks'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:scheduled-tasks' },
    [ordered]@{ canonical_name = 'Get-Container'; aliases = @(); module = 'Containers'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:containers' },
    [ordered]@{ canonical_name = 'Get-NetAdapter'; aliases = @(); module = 'NetTCPIP'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:networking' },
    [ordered]@{ canonical_name = 'Get-Volume'; aliases = @(); module = 'Storage'; kind = 'cmdlet'; hosts = @('Desktop51'); behavior = 'table'; strategy = 'specialized:storage' }
)

$json = $entries | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText((Join-Path (Get-Location) $Output), "$json`n", [System.Text.UTF8Encoding]::new($false))
Write-Output "Wrote offline PowerShell manifest snapshot to $Output"
