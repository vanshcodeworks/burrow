//! Property tests for atomic snapshot creation and restore.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use burrow_data::{create_snapshot, list_snapshots, restore_snapshot, snapshot_dir, snapshots_dir};
use proptest::prelude::*;

type Files = BTreeMap<String, Vec<u8>>;

fn write_files(root: &Path, files: &Files) {
    fs::create_dir_all(root).expect("create data directory");
    for (relative, bytes) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(&path, bytes).expect("write file");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn snapshot_restore_round_trips_and_leaves_no_staging(
        services in prop::collection::btree_map(
            "svc[a-z0-9]{0,4}",
            prop::option::of(prop::collection::btree_map(
                prop_oneof![
                    "f[a-z0-9]{0,5}",
                    "nested/[fd][a-z0-9]{0,5}",
                ],
                prop::collection::vec(any::<u8>(), 0..48),
                0..6,
            )),
            1..4,
        ),
    ) {
        let temp = tempfile::tempdir().expect("temp dir");
        let burrow = temp.path().join(".burrow");
        let data = burrow.join("data");
        for (service, files) in &services {
            if let Some(files) = files {
                write_files(&data.join(service), files);
            }
        }

        let service_names: Vec<String> = services.keys().cloned().collect();
        let manifest = create_snapshot(&burrow, "roundtrip", &service_names)
            .expect("create snapshot");
        prop_assert_eq!(manifest.services.len(), services.len());
        for captured in &manifest.services {
            let files = services[&captured.name].as_ref();
            prop_assert_eq!(captured.present, files.is_some());
            prop_assert_eq!(
                captured.files,
                files.map_or(0, |files| files.len() as u64)
            );
            prop_assert_eq!(
                captured.bytes,
                files.map_or(0, |files| {
                    files.values().map(|bytes| bytes.len() as u64).sum()
                })
            );
        }

        // Change the live data after the snapshot: add a file and remove one.
        for (service, files) in &services {
            if let Some(files) = files {
                let directory = data.join(service);
                fs::write(directory.join("zzz-changed.txt"), b"changed")
                    .expect("add changed file");
                if let Some(first) = files.keys().next() {
                    fs::remove_file(directory.join(first)).expect("remove original file");
                }
            }
        }

        restore_snapshot(&burrow, "roundtrip").expect("restore");

        for (service, files) in &services {
            let directory = data.join(service);
            if let Some(files) = files {
                for (relative, expected) in files {
                    let actual = fs::read(directory.join(relative)).expect("restored file");
                    prop_assert_eq!(&actual, expected);
                }
                prop_assert!(
                    !directory.join("zzz-changed.txt").exists(),
                    "a post-snapshot file survived the restore"
                );
            } else {
                prop_assert!(
                    !directory.exists(),
                    "a service recorded as absent was created by restore"
                );
            }
        }

        // A completed restore must remove its staging, backups, and journal.
        for entry in fs::read_dir(&data).expect("read data root") {
            let name = entry.expect("entry").file_name();
            let name = name.to_string_lossy();
            prop_assert!(!name.starts_with(".burrow-restore-"), "staging left: {name}");
            prop_assert!(!name.starts_with(".burrow-backup-"), "backup left: {name}");
        }
        for entry in fs::read_dir(snapshots_dir(&burrow)).expect("read snapshots") {
            let name = entry.expect("entry").file_name();
            let name = name.to_string_lossy();
            prop_assert!(!name.starts_with(".burrow-tmp-"), "temporary snapshot left: {name}");
        }
        prop_assert!(snapshot_dir(&burrow, "roundtrip").is_dir());
        prop_assert_eq!(list_snapshots(&burrow).expect("list").len(), 1);
    }
}
