//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

/// A serializable struct that is backed by a JSON file on disk.
///
/// The disk version will be loaded on object creation. If it does not exist,
/// or is invalid, the default value will be returned.
pub struct DiskBacked<T: Eq + Serialize + Deserialize<'static>> {
    path: PathBuf,
    cache: Option<T>,
    default: T,
}

#[derive(Debug, Eq, PartialEq)]
pub enum UpdateStatus {
    Unchanged,
    Updated,
}

impl<T: Default + Eq + Serialize + DeserializeOwned> DiskBacked<T> {
    /// New instance from a path on disk
    pub fn from_path(path: &Path) -> Self {
        Self::from_path_with_default(path, T::default())
    }
}

impl<T: Eq + Serialize + DeserializeOwned> DiskBacked<T> {
    pub fn from_path_with_default(path: &Path, default: T) -> Self {
        let cache = match File::open(path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                serde_json::from_reader(reader).ok()
            }
            Err(_) => None,
        };
        Self {
            path: path.to_owned(),
            cache,
            default,
        }
    }

    /// Return current value (disk value or default).
    pub fn get(&self) -> &T {
        match self.cache.as_ref() {
            Some(v) => v,
            None => &self.default,
        }
    }

    /// Updates self and writes the provided value to disk.
    pub fn set(&mut self, new_value: T) -> Result<UpdateStatus> {
        let has_changed_from_previous_effective_value = match self.cache.as_ref() {
            Some(v) => new_value != *v,
            None => new_value != self.default,
        };
        let needs_writing = match &self.cache {
            Some(v) => v != &new_value,
            None => true,
        };

        if needs_writing {
            let file = File::create(&self.path)?;
            serde_json::to_writer_pretty(file, &new_value)?;
            self.cache = Some(new_value);
        }

        Ok(match has_changed_from_previous_effective_value {
            true => UpdateStatus::Updated,
            false => UpdateStatus::Unchanged,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, io::BufReader, path::PathBuf};

    use rstest::{fixture, rstest};

    use crate::test_utils::create_file_with_contents;

    use super::*;

    #[rstest]
    fn test_defaults_to_default(fixture: Fixture) {
        assert_eq!(
            *DiskBacked::<TestJson>::from_path(&fixture.path).get(),
            TestJson::default()
        );
    }

    #[rstest]
    fn test_load_from_disk(#[with(Some(TEST1))] fixture: Fixture) {
        let config = DiskBacked::<TestJson>::from_path(&fixture.path);
        assert_eq!(*config.get(), TEST1);
    }

    #[rstest]
    fn test_write_with_no_existing_file_and_new_equals_default(#[with(None)] fixture: Fixture) {
        let mut config = DiskBacked::<TestJson>::from_path(&fixture.path);

        let result = config.set(TestJson::default());

        assert!(matches!(result, Ok(UpdateStatus::Unchanged)));

        // We should have created a file on disk and stored the config
        assert_eq!(fixture.read_config(), Some(TestJson::default()));
    }

    #[rstest]
    fn test_write_with_no_existing_file_and_new_is_not_default(#[with(None)] fixture: Fixture) {
        let mut config = DiskBacked::<TestJson>::from_path(&fixture.path);

        let result = config.set(TEST1);

        assert!(matches!(result, Ok(UpdateStatus::Updated)));

        // We should have created a file on disk and stored the config
        assert_eq!(fixture.read_config(), Some(TEST1));
    }

    #[rstest]
    fn test_write_with_corrupted_local_file(#[with(None)] fixture: Fixture) {
        create_file_with_contents(&fixture.path, "DIS*IS*NOT*JSON".as_bytes()).unwrap();
        let mut config = DiskBacked::<TestJson>::from_path(&fixture.path);

        let result = config.set(TestJson::default());

        // Unchanged because we use default when file is corrupted
        assert!(matches!(result, Ok(UpdateStatus::Unchanged)));

        // We should have created a file on disk and stored the config
        assert_eq!(fixture.read_config(), Some(TestJson::default()));
    }

    #[rstest]
    fn test_write_without_change(#[with(Some(TEST1))] fixture: Fixture) {
        let mut config = DiskBacked::<TestJson>::from_path(&fixture.path);

        // Delete the config file so we can see if it has been recreated
        std::fs::remove_file(&fixture.path).unwrap();

        let result = config.set(TEST1);

        assert!(matches!(result, Ok(UpdateStatus::Unchanged)));

        // We should NOT have re-created a file on disk and stored the config
        assert_eq!(fixture.read_config(), None);
    }

    #[rstest]
    fn test_write_with_change(#[with(Some(TEST1))] fixture: Fixture) {
        let mut config = DiskBacked::<TestJson>::from_path(&fixture.path);

        // Delete the config file so we can see if it has been recreated
        std::fs::remove_file(&fixture.path).unwrap();

        let result = config.set(TestJson::default());

        assert!(matches!(result, Ok(UpdateStatus::Updated)));

        assert_eq!(fixture.read_config(), Some(TestJson::default()));
    }

    #[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Default, Debug)]
    struct TestJson<'a> {
        pub message: Cow<'a, str>,
    }

    const TEST1: TestJson = TestJson {
        message: Cow::Borrowed("test1"),
    };

    #[fixture]
    fn fixture(#[default(None)] config: Option<TestJson>) -> Fixture {
        Fixture::new(config)
    }

    struct Fixture {
        _temp_dir: tempfile::TempDir,
        path: PathBuf,
    }

    impl Fixture {
        fn new(preexisting: Option<TestJson>) -> Self {
            let temp_dir = tempfile::tempdir().unwrap();
            let path = temp_dir.path().join("data.json");

            if let Some(value) = preexisting {
                let file = File::create(&path).unwrap();
                serde_json::to_writer_pretty(file, &value).unwrap();
            }

            Self {
                _temp_dir: temp_dir,
                path,
            }
        }

        fn read_config(&self) -> Option<TestJson> {
            let file = match File::open(&self.path) {
                Ok(file) => file,
                Err(_) => return None,
            };
            let reader = BufReader::new(file);
            match serde_json::from_reader(reader) {
                Ok(config) => Some(config),
                Err(_) => None,
            }
        }
    }
}
