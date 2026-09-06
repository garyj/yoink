use yoink_core::{Config, ConfigError, DEFAULT_MAX_ITEMS};

#[test]
fn missing_file_gives_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::load(&dir.path().join("config.toml")).unwrap();
    assert_eq!(config, Config::default());
    assert_eq!(config.max_items, DEFAULT_MAX_ITEMS);
}

#[test]
fn empty_file_gives_defaults() {
    assert_eq!(Config::parse("").unwrap(), Config::default());
}

#[test]
fn reads_max_items_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# keep fewer\nmax_items = 12\n").unwrap();
    assert_eq!(Config::load(&path).unwrap().max_items, 12);
}

#[test]
fn unknown_key_is_an_error() {
    let err = Config::parse("max_item = 12\n").unwrap_err();
    assert!(matches!(err, ConfigError::Parse(_)));
    assert!(err.to_string().contains("max_item"), "{err}");
}

#[test]
fn negative_max_items_is_an_error() {
    assert!(matches!(
        Config::parse("max_items = -1\n").unwrap_err(),
        ConfigError::Parse(_)
    ));
}

#[test]
fn unreadable_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where the file should be: read_to_string fails with a non-NotFound error.
    let path = dir.path().join("config.toml");
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(
        Config::load(&path).unwrap_err(),
        ConfigError::Read(_)
    ));
}
