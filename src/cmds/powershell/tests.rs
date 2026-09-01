//! Cross-module PowerShell tests live here; parser unit tests stay beside the parser.

#[cfg(test)]
mod adapter_tests {
    use super::super::adapters;

    #[test]
    fn generic_table_filter_preserves_headers_and_rows() {
        let raw =
            "Name        Status\n----        ------\nalpha       Running\nbeta        Stopped\n";
        let filtered = adapters::filter_output("generic", raw).expect("table is filterable");

        assert!(filtered.len() < raw.len());
        assert!(filtered.contains("Name=alpha"));
        assert!(filtered.contains("Status=Stopped"));
    }

    #[test]
    fn malformed_and_single_line_output_falls_back_to_native() {
        assert!(adapters::filter_output("generic", "one line\n").is_none());
        assert!(adapters::filter_output("generic", "Name\n----\n").is_none());
    }

    #[test]
    fn filesystem_table_preserves_directory_context() {
        let raw = "Directory: C:\\work\\reports\n\nMode        LastWriteTime        Length        Name\n----        -------------        ------        ----\n-a--        2026-09-01 10:00     42             report.txt\n";

        let filtered = adapters::filter_output("filesystem", raw).expect("table is filterable");

        assert!(filtered.contains("Directory: C:\\work\\reports"));
        assert!(filtered.contains("Name=report.txt"));
        assert!(filtered.len() < raw.len());
    }

    #[test]
    fn lossy_utf8_replacement_is_never_compacted() {
        let raw = "Name        Status\n----        ------\nalpha       \u{fffd}\n";

        assert!(adapters::filter_output("generic", raw).is_none());
    }

    #[test]
    fn list_filter_preserves_unicode_values() {
        let raw = "Name : \u{041f}\u{0440}\u{043e}\u{0434}\nStatus : Running\n\n";
        let filtered = adapters::filter_output("generic", raw).expect("list is filterable");

        assert!(filtered.contains("\u{041f}\u{0440}\u{043e}\u{0434}"));
        assert!(filtered.len() < raw.len());
    }
}
