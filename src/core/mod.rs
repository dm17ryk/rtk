//! Building blocks shared across all RTK modules.

pub mod ai_output;
pub mod args_utils;
pub mod config;
pub mod constants;
pub mod display_helpers;
pub mod filter;
pub mod guard;
pub mod path_inventory;
pub mod runner;
pub mod stream;
pub mod tee;
pub mod telemetry;
pub mod telemetry_cmd;
pub mod toml_filter;
pub mod tracking;
pub mod truncate;
pub mod utils;

#[cfg(test)]
mod path_inventory_tests {
    use super::ai_output::{render, BudgetClass, Omission};
    use super::path_inventory::{canonical_groups, common_root, document};

    #[test]
    fn canonical_groups_elides_a_shared_root_and_sorts_leaves() {
        let paths = vec![
            "src/core/tracking.rs".to_string(),
            "src/cmds/read.rs".to_string(),
            "src/core/runner.rs".to_string(),
        ];

        assert_eq!(
            canonical_groups(&paths),
            vec![
                ("cmds".to_string(), vec!["read.rs".to_string()]),
                (
                    "core".to_string(),
                    vec!["runner.rs".to_string(), "tracking.rs".to_string()],
                ),
            ]
        );
    }

    #[test]
    fn canonical_groups_normalizes_windows_path_separators() {
        let paths = vec![
            "src\\core\\runner.rs".to_string(),
            "src\\core\\tracking.rs".to_string(),
        ];

        assert_eq!(
            canonical_groups(&paths),
            vec![(
                ".".to_string(),
                vec!["runner.rs".to_string(), "tracking.rs".to_string()],
            )]
        );
    }

    #[test]
    fn common_root_elides_the_deepest_shared_directory() {
        let paths = vec![
            "C:\\Temp\\project\\inventory\\one.txt".to_string(),
            "C:\\Temp\\project\\inventory\\two.txt".to_string(),
        ];

        assert_eq!(
            common_root(&paths),
            Some("C:/Temp/project/inventory".to_string())
        );
        assert_eq!(
            canonical_groups(&paths),
            vec![(
                ".".to_string(),
                vec!["one.txt".to_string(), "two.txt".to_string()],
            )]
        );
    }

    #[test]
    fn shared_directory_inventory_is_smaller_than_native_paths() {
        let paths = (0..80)
            .map(|index| format!("D:/Temp/project/inventory/item-{index:03}.txt"))
            .collect::<Vec<_>>();
        let native = format!("{}\n", paths.join("\n"));
        let rendered = render(&document(&paths), BudgetClass::Collection).text;

        assert!(
            crate::core::tracking::estimate_tokens(&rendered)
                < crate::core::tracking::estimate_tokens(&native),
            "native={native:?}\nrendered={rendered:?}"
        );
    }

    #[test]
    fn inventory_document_emits_root_once_with_grouped_relative_paths() {
        let paths = vec![
            "src/core/tracking.rs".to_string(),
            "src/cmds/read.rs".to_string(),
            "src/core/runner.rs".to_string(),
        ];

        assert_eq!(
            render(&document(&paths), BudgetClass::Collection).text,
            "status=inventory files=3 dirs=2 root=src\ncmds/{read.rs}\ncore/{runner.rs,tracking.rs}"
        );
    }

    #[test]
    fn inventory_document_keeps_disjoint_roots_explicit() {
        let paths = vec!["src/main.rs".to_string(), "tests/read.rs".to_string()];

        assert_eq!(
            render(&document(&paths), BudgetClass::Collection).text,
            "status=inventory files=2 dirs=2\nsrc/{main.rs}\ntests/{read.rs}"
        );
    }

    #[test]
    fn inventory_document_counts_omitted_files_not_directory_records() {
        let paths = (0..2_000)
            .map(|n| format!("src/core/file-{n:04}.rs"))
            .collect::<Vec<_>>();

        assert_eq!(
            render(&document(&paths), BudgetClass::Collection).omission,
            Some(Omission {
                items: 2_000,
                groups: 1,
            })
        );
    }
}
