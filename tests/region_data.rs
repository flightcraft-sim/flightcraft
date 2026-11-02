// Shared only by tests that exercise optional installed region data.
macro_rules! require_region_data {
    () => {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain");
        if !root.join("new-york/000.fcht").is_file()
            || !root.join("eastern-pa/000.fcht").is_file()
        {
            assert!(std::env::var_os("CI").is_none(), "CI requires region tiles: run cargo xtask data fetch");
            eprintln!("Skipping installed-region test: data absent locally; run cargo xtask data fetch (or data fetch --offline to restore the cache).");
            return;
        }
    };
}
