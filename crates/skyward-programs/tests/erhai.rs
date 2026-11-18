//! The Qualcomm service layout observed on the Pad.
use skyward_programs::phone::{Partition, Phone};
use skyward_programs::secure::{self, Step};
use std::collections::HashMap;
use std::path::Path;

fn pad(gatekeeper: Option<&str>) -> Phone {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/erhai-stock");
    let partitions = ["system_ext", "vendor", "odm"]
        .iter()
        .map(|name| Partition {
            name: (*name).into(),
            mounted: match *name {
                "vendor" | "odm" => format!("/{name}").into(),
                _ => format!("/mnt/{name}").into(),
            },
            at: root.join(name),
        })
        .collect();
    let mut properties: HashMap<String, String> = [
        ("ro.boot.hardware", "qcom"),
        ("ro.boot.product.vendor.sku", "sun"),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value.into()))
    .collect();
    // The caller reads installed build.prop values before discovering HALs.
    let build = std::fs::read_to_string(root.join("vendor/build.prop")).unwrap();
    properties.extend(
        build
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, value)| (name.into(), value.into())),
    );
    if let Some(value) = gatekeeper {
        properties.insert(
            "vendor.gatekeeper.is_security_level_spu".into(),
            value.into(),
        );
    }
    Phone::read(partitions, properties, None).unwrap()
}

#[test]
fn pad_uses_its_qualcomm_keys_without_a_phone_weaver_chain() {
    let phone = pad(None);
    let steps = secure::steps(&phone).unwrap();
    let programs = secure::programs(&steps);
    let names: Vec<&str> = programs.iter().map(|p| p.service.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "/vendor/bin/qseecomd",
            "/vendor/bin/ssgtzd",
            "/vendor/bin/hlosminkdaemon",
            "/vendor/bin/hw/android.hardware.security.keymint-service-qti",
            "/vendor/bin/hw/android.hardware.gatekeeper-service-qti",
            "/vendor/bin/hw/android.hardware.boot-service.qti",
        ]
    );
    assert!(!steps.iter().any(|step| matches!(step, Step::ServeElements)));
    assert!(
        !phone
            .told_of()
            .iter()
            .any(|name| name.contains("weaver") || name.contains("nfc"))
    );
    assert_eq!(
        programs[0].service.ready_property.as_deref(),
        Some("vendor.sys.listeners.registered")
    );
    assert_eq!(
        programs[4].service.ready_service.as_deref(),
        Some("android.hardware.gatekeeper.IGatekeeper/default")
    );
    assert_eq!(programs[4].service.user.as_deref(), Some("system"));
    assert_eq!(programs[4].service.groups, ["system"]);
    let keymint = &programs[3].service;
    assert!(
        keymint
            .registers
            .iter()
            .any(|name| name == "android.hardware.security.sharedsecret.ISharedSecret/default")
    );
}

#[test]
fn gatekeeper_selection_follows_the_platform_property() {
    let steps = secure::steps(&pad(Some("1"))).unwrap();
    let programs = secure::programs(&steps);
    assert_eq!(
        programs[4].service.path,
        "/vendor/bin/hw/android.hardware.gatekeeper-service-spu-qti"
    );
}
