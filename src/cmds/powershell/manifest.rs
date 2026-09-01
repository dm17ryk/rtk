//! Checked-in, offline PowerShell capability snapshots.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostVersion {
    Desktop51,
    Pwsh74,
    Pwsh75,
    Pwsh76,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BehaviorClass {
    Table,
    List,
    Identity,
    Machine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestEntry {
    pub canonical_name: &'static str,
    pub aliases: &'static [&'static str],
    pub module: &'static str,
    pub kind: &'static str,
    pub hosts: &'static [HostVersion],
    pub behavior: BehaviorClass,
    pub strategy: &'static str,
}

const WINDOWS_HOSTS: &[HostVersion] = &[HostVersion::Desktop51];
const ALL_PWSH: &[HostVersion] = &[
    HostVersion::Pwsh74,
    HostVersion::Pwsh75,
    HostVersion::Pwsh76,
];
const ALL_HOSTS: &[HostVersion] = &[
    HostVersion::Desktop51,
    HostVersion::Pwsh74,
    HostVersion::Pwsh75,
    HostVersion::Pwsh76,
];

/// Representative inbox and client-capability commands.  The generator that
/// produced this snapshot is intentionally offline; runtime never downloads
/// module metadata.
pub const SNAPSHOT: &[ManifestEntry] = &[
    ManifestEntry {
        canonical_name: "Get-ChildItem",
        aliases: &["gci", "ls", "dir"],
        module: "Microsoft.PowerShell.Management",
        kind: "cmdlet",
        hosts: ALL_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:filesystem",
    },
    ManifestEntry {
        canonical_name: "Get-Process",
        aliases: &["gps", "ps"],
        module: "Microsoft.PowerShell.Management",
        kind: "cmdlet",
        hosts: ALL_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:process",
    },
    ManifestEntry {
        canonical_name: "Get-Service",
        aliases: &["gsv"],
        module: "Microsoft.PowerShell.Management",
        kind: "cmdlet",
        hosts: ALL_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:service",
    },
    ManifestEntry {
        canonical_name: "Get-Command",
        aliases: &["gcm"],
        module: "Microsoft.PowerShell.Core",
        kind: "cmdlet",
        hosts: ALL_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:discovery",
    },
    ManifestEntry {
        canonical_name: "Get-Module",
        aliases: &["gmo"],
        module: "Microsoft.PowerShell.Core",
        kind: "cmdlet",
        hosts: ALL_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:discovery",
    },
    ManifestEntry {
        canonical_name: "Get-CimInstance",
        aliases: &["gcim"],
        module: "CimCmdlets",
        kind: "cmdlet",
        hosts: ALL_PWSH,
        behavior: BehaviorClass::Table,
        strategy: "specialized:cim",
    },
    ManifestEntry {
        canonical_name: "Get-ADUser",
        aliases: &[],
        module: "ActiveDirectory",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:active-directory",
    },
    ManifestEntry {
        canonical_name: "Get-VM",
        aliases: &[],
        module: "Hyper-V",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:hyper-v",
    },
    ManifestEntry {
        canonical_name: "Get-MpComputerStatus",
        aliases: &[],
        module: "Defender",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:defender",
    },
    ManifestEntry {
        canonical_name: "Get-BitLockerVolume",
        aliases: &[],
        module: "BitLocker",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:bitlocker",
    },
    ManifestEntry {
        canonical_name: "Get-ScheduledTask",
        aliases: &[],
        module: "ScheduledTasks",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:scheduled-tasks",
    },
    ManifestEntry {
        canonical_name: "Get-Container",
        aliases: &[],
        module: "Containers",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:containers",
    },
    ManifestEntry {
        canonical_name: "Get-NetAdapter",
        aliases: &[],
        module: "NetTCPIP",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:networking",
    },
    ManifestEntry {
        canonical_name: "Get-Volume",
        aliases: &[],
        module: "Storage",
        kind: "cmdlet",
        hosts: WINDOWS_HOSTS,
        behavior: BehaviorClass::Table,
        strategy: "specialized:storage",
    },
];

pub fn entries_for(host: HostVersion) -> impl Iterator<Item = &'static ManifestEntry> {
    SNAPSHOT
        .iter()
        .filter(move |entry| entry.hosts.contains(&host))
}

pub fn validate() -> Result<(), &'static str> {
    for (index, entry) in SNAPSHOT.iter().enumerate() {
        if entry.canonical_name.is_empty() || entry.module.is_empty() || entry.strategy.is_empty() {
            return Err("manifest entries require name, module, and strategy");
        }
        if SNAPSHOT[..index].iter().any(|prior| {
            prior
                .canonical_name
                .eq_ignore_ascii_case(entry.canonical_name)
        }) {
            return Err("manifest canonical names must be unique");
        }
        if entry
            .aliases
            .iter()
            .enumerate()
            .any(|(alias_index, alias)| {
                alias.is_empty()
                    || alias.eq_ignore_ascii_case(entry.canonical_name)
                    || entry.aliases[..alias_index]
                        .iter()
                        .any(|prior_alias| prior_alias.eq_ignore_ascii_case(alias))
                    || SNAPSHOT[..index].iter().any(|prior| {
                        prior.canonical_name.eq_ignore_ascii_case(alias)
                            || prior
                                .aliases
                                .iter()
                                .any(|prior_alias| prior_alias.eq_ignore_ascii_case(alias))
                    })
            })
        {
            return Err("manifest aliases must be unique and non-empty");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_unique_and_has_strategy_for_every_entry() {
        validate().expect("valid checked-in manifest");
        assert!(entries_for(HostVersion::Desktop51).count() >= 5);
        assert!(entries_for(HostVersion::Pwsh76).count() >= 5);
        let checked_in: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("manifest_snapshot.json")).expect("JSON snapshot");
        assert_eq!(checked_in.len(), SNAPSHOT.len());
    }
}
