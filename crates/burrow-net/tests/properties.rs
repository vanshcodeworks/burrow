//! Property tests for stable, collision-free port allocation.

use std::collections::BTreeMap;

use burrow_net::{PortAllocator, AUTO_PORT_END, AUTO_PORT_START};
use proptest::prelude::*;

proptest! {
    #[test]
    fn allocations_are_unique_in_range_and_stable(
        names in prop::collection::btree_set("svc[a-z0-9]{0,5}", 1..8),
    ) {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut allocator = PortAllocator::load(temp.path()).expect("load");
        let mut assigned = BTreeMap::new();

        for name in &names {
            let port = allocator.allocate(name, None).expect("allocate");
            prop_assert!(
                (AUTO_PORT_START..AUTO_PORT_END).contains(&port),
                "allocated {port} outside {AUTO_PORT_START}-{}",
                AUTO_PORT_END - 1
            );
            prop_assert!(
                !assigned.values().any(|seen| *seen == port),
                "port {port} was handed to two services"
            );
            assigned.insert(name.clone(), port);
        }

        // Re-allocating an unchanged service must return the remembered port.
        for (name, port) in &assigned {
            prop_assert_eq!(allocator.allocate(name, None).expect("reuse"), *port);
            prop_assert_eq!(allocator.remembered(name), Some(*port));
        }

        // The assignments survive a save/load cycle, and the atomic write
        // leaves no temporary file behind.
        allocator.save().expect("save");
        prop_assert!(!temp.path().join("ports.json.tmp").exists());
        let reloaded = PortAllocator::load(temp.path()).expect("reload");
        for (name, port) in &assigned {
            prop_assert_eq!(reloaded.remembered(name), Some(*port));
        }
    }

    #[test]
    fn reserved_and_pinned_ports_are_never_handed_out(
        reserved in any::<u16>(),
        names in prop::collection::btree_set("svc[a-z0-9]{0,5}", 1..6),
    ) {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut allocator = PortAllocator::load(temp.path()).expect("load");
        allocator.reserve(reserved);
        allocator.pin("pinned", reserved);

        for name in &names {
            let port = allocator.allocate(name, None).expect("allocate");
            prop_assert_ne!(port, reserved, "reserved port was handed out");
        }
        prop_assert_eq!(
            allocator.allocate("pinned", None).expect("pinned"),
            reserved,
            "a pinned service must keep its port even when it is busy"
        );
    }
}
