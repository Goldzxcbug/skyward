use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use skyward_config::ramdisk as paths;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

mod avb;
mod described;

use skyward_bootimg::{BootImage, Ramdisk};

/// Builds Skyward recovery images from a device's own stock images.
#[derive(Parser)]
#[command(name = "skyward-pack")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print an image's header and the files in its ramdisk.
    List { image: PathBuf },
    /// Write one ramdisk file to stdout.
    Cat { image: PathBuf, path: String },
    /// Unpack and pack an image's ramdisk unchanged, to check that a rebuilt
    /// archive keeps every entry.
    Repack { image: PathBuf, out: PathBuf },
    /// Join a kernel and another image's ramdisk into one boot image.
    Boot {
        /// Image holding the kernel (the device's boot partition).
        #[arg(long)]
        kernel: PathBuf,
        /// Image holding the ramdisk.
        #[arg(long)]
        ramdisk: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Build a Skyward recovery image on top of the device's stock recovery:
    /// its init, adbd, SELinux policy and libraries stay, the recovery
    /// service becomes Skyward. The image fills the whole partition and ends
    /// in an unsigned AVB footer of its own.
    Recovery {
        /// The device's stock recovery image: the whole partition, as the
        /// phone or a full update package holds it, ending in its AVB footer.
        #[arg(long)]
        stock: PathBuf,
        /// Skyward built for the device (aarch64-linux-android).
        #[arg(long)]
        binary: PathBuf,
        /// The first program of a system installed beside the installed
        /// one, skyward-system-init built for the device and linked
        /// statically.
        #[arg(long)]
        system_init: PathBuf,
        /// The device configuration, devices/<codename>.toml.
        #[arg(long)]
        config: PathBuf,
        /// The UI typeface.
        #[arg(long)]
        font: PathBuf,
        /// The terminal's monospaced typeface.
        #[arg(long)]
        mono: PathBuf,
        /// The monospaced typeface's licence, which goes with it.
        #[arg(long)]
        mono_license: PathBuf,
        /// Omit the replaced Oplus recovery UI's resources to make room for
        /// a full typeface. No executables, policies or libraries are removed.
        #[arg(long)]
        omit_oplus_ui: bool,
        #[arg(long)]
        out: PathBuf,
    },
    /// Print the stock build a device's recovery is made on, as lines of
    /// name=value: its version, where its full update package is, and the
    /// SHA-256 of the recovery partition in that package. Given somewhere
    /// to write it, also take that partition out of the package on its
    /// server, reading only what the partition takes, and write it there
    /// once its SHA-256 is that one.
    Stock {
        /// The device configuration, devices/<codename>.toml.
        #[arg(long)]
        config: PathBuf,
        /// Where to write the stock recovery image.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Take what a device's own partitions say of the programs they carry
    /// out of its stock build's full update package on its server: the
    /// init scripts, the service descriptions, the accounts and the build
    /// properties of each partition that carries such programs, written
    /// under `out` as the partition and then the path in it. Only the
    /// parts of the package that hold these files are downloaded, each
    /// checked against the package's own sum of it.
    Described {
        /// The device configuration, devices/<codename>.toml.
        #[arg(long)]
        config: PathBuf,
        /// A partition to take them out of; vendor, odm and system_ext
        /// when none is named.
        #[arg(long = "partition")]
        partitions: Vec<String>,
        /// The folder to write the files under, which is not there yet.
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::List { image } => list(&image),
        Cmd::Cat { image, path } => {
            let rd = Ramdisk::decode(&BootImage::read(&image)?.ramdisk)?;
            let e = rd
                .find(&path)
                .with_context(|| format!("{path} is not in the ramdisk"))?;
            std::io::stdout().write_all(&e.data)?;
            Ok(())
        }
        Cmd::Repack { image, out } => {
            let mut img = BootImage::read(&image)?;
            img.ramdisk = Ramdisk::decode(&img.ramdisk)?.encode()?;
            write(&out, &img, &img.to_bytes()?)
        }
        Cmd::Boot {
            kernel,
            ramdisk,
            out,
        } => {
            let mut img = BootImage::read(&ramdisk)?;
            img.kernel = BootImage::read(&kernel)?.kernel;
            write(&out, &img, &img.to_bytes()?)
        }
        Cmd::Recovery {
            stock,
            binary,
            system_init,
            config,
            font,
            mono,
            mono_license,
            omit_oplus_ui,
            out,
        } => {
            let size = partition_size(&stock)?;
            // The bootloader refuses an image whose vbmeta names a smaller
            // rollback index than the one it keeps for the slot, unlocked or
            // not, so the stock image's own is kept.
            let rollback = rollback(&stock)?;
            let mut img = BootImage::read(&stock)?;
            let mut rd = Ramdisk::decode(&img.ramdisk)?;
            // A stock recovery ramdisk has no third-party tree; refuse to build
            // on top of another custom recovery.
            if let Some(e) = rd
                .entries
                .iter()
                .find(|e| e.name == "FFiles" || e.name.starts_with("twres"))
            {
                anyhow::bail!(
                    "{} holds {}, which is not a stock recovery",
                    stock.display(),
                    e.name
                );
            }
            let read =
                |p: &PathBuf| std::fs::read(p).with_context(|| format!("read {}", p.display()));
            let cfg = skyward_config::DeviceConfig::load(&config)?;
            let oplus = prepare_oplus_init(&mut rd, omit_oplus_ui)?;
            rd.replace(paths::RECOVERY, read(&binary)?)?;
            if omit_oplus_ui {
                rd.entries
                    .retain(|e| e.name != "res/OplusOS" && !e.name.starts_with("res/OplusOS/"));
            }
            // Install scripts look for the commands a stock recovery lacks
            // among its programs, and its unzip there too, a link to ziptool
            // until it points here; the recovery binary is each of them.
            let (_, recovery) = paths::RECOVERY
                .rsplit_once('/')
                .expect("the recovery binary is in a folder");
            for command in paths::COMMANDS {
                rd.link(command, recovery)?;
            }
            // Diagnostics must use this name: init's `recovery` entry always
            // starts the service, even when extra arguments are supplied.
            rd.link("system/bin/skyward", recovery)?;
            rd.add(paths::SYSTEM_INIT, 0o755, read(&system_init)?)?;
            rd.add(paths::CONFIG, 0o644, read(&config)?)?;
            rd.add(paths::FONT, 0o644, read(&font)?)?;
            rd.add(paths::MONO, 0o644, read(&mono)?)?;
            rd.add(paths::MONO_LICENSE, 0o644, read(&mono_license)?)?;
            // A stock ramdisk may run logd without any scheduling profiles,
            // and logd stops as soon as it cannot find the one it asks for.
            if rd.find(paths::TASK_PROFILES).is_none() {
                rd.add(paths::TASK_PROFILES, 0o644, TASK_PROFILES.to_vec())?;
            }
            // Oplus recovery does not define config_disable. Its absent
            // property must not prevent adbd starting for MTP.
            let mtp = if oplus {
                String::from_utf8_lossy(MTP_RC)
                    .replace(" && property:sys.usb.config_disable=0", "")
                    .into_bytes()
            } else {
                MTP_RC.to_vec()
            };
            rd.add(paths::MTP_RC, 0o644, mtp)?;
            // On an unlocked device adbd keeps root when ro.secure is 0 and
            // lets any computer in without authorization when ro.adb.secure is
            // 0; init lets a later line of a property file override an earlier
            // one. update_engine refuses a package built before the recovery
            // itself unless both of the last two are set; the recovery checks
            // packages against the installed system instead.
            let stock_props = &rd
                .find(paths::PROP_DEFAULT)
                .with_context(|| format!("{} is not in the ramdisk", paths::PROP_DEFAULT))?
                .data;
            let mut props = without(stock_props, &BUILD_VERSION);
            if oplus {
                props = device_identity(&props, &cfg.device.codename);
            }
            props.extend_from_slice(
                b"\nro.secure=0\nro.adb.secure=0\nro.debuggable=1\nro.ota.allow_downgrade=true\n",
            );
            rd.replace(paths::PROP_DEFAULT, props)?;
            img.ramdisk = rd.encode()?;
            let bytes = img.to_bytes()?;
            write(
                &out,
                &img,
                &avb::footed(&bytes, "recovery", size, &Sha256::digest(&bytes), rollback)?,
            )
        }
        Cmd::Stock { config, out } => {
            let stock = skyward_config::DeviceConfig::load(&config)?
                .stock
                .with_context(|| format!("{} names no stock build", config.display()))?;
            println!(
                "version={}\npackage={}\nrecovery={}",
                stock.version, stock.package, stock.recovery
            );
            match out {
                Some(out) => take_stock_recovery(&stock, &out),
                None => Ok(()),
            }
        }
        Cmd::Described {
            config,
            partitions,
            out,
        } => {
            let stock = skyward_config::DeviceConfig::load(&config)?
                .stock
                .with_context(|| format!("{} names no stock build", config.display()))?;
            let partitions = if partitions.is_empty() {
                skyward_programs::phone::PARTITIONS
                    .map(str::to_string)
                    .to_vec()
            } else {
                partitions
            };
            described::take(&stock.package, &partitions, &out)
        }
    }
}

/// The partition a recovery is made on, as a full update package names it.
const RECOVERY: &str = "recovery";

/// Take the recovery partition out of the stock build's full update package
/// on its server, reading only what that partition takes of it, and write it
/// to `out` once its SHA-256 is the one the configuration names.
fn take_stock_recovery(stock: &skyward_config::Stock, out: &Path) -> Result<()> {
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).with_context(|| format!("make {}", parent.display()))?;
    // Taken out beside where it goes, so it is moved there whole.
    let work = tempfile::Builder::new()
        .prefix(".stock")
        .tempdir_in(parent)
        .with_context(|| format!("make a folder in {}", parent.display()))?;
    let payload = unpayload::Payload::open_url(&stock.package)
        .with_context(|| format!("open {}", stock.package))?;
    let options = unpayload::ExtractOptions {
        output_dir: work.path().to_path_buf(),
        partitions: vec![RECOVERY.into()],
        ..Default::default()
    };
    let report = |event: unpayload::Event<'_>| match event {
        unpayload::Event::Started { partitions } => {
            for (name, bytes) in partitions {
                println!("taking {name} out of the package: {bytes} bytes");
            }
        }
        unpayload::Event::Extracted(image) => {
            println!("took {} out: {} bytes", image.partition, image.size)
        }
        unpayload::Event::Progress { .. } => {}
    };
    let taken = payload
        .extract(&options, &report)
        .with_context(|| format!("take {RECOVERY} out of {}", stock.package))?;
    let image = taken
        .iter()
        .find(|i| i.partition == RECOVERY)
        .with_context(|| format!("{} holds no {RECOVERY} partition", stock.package))?;
    let sum = hex::encode(
        image
            .sha256
            .as_deref()
            .context("the package's own sum of the image was not checked")?,
    );
    ensure!(
        sum == stock.recovery,
        "the {RECOVERY} partition in {} has the SHA-256 {sum}, where the configuration names {}",
        stock.package,
        stock.recovery
    );
    std::fs::rename(&image.path, out)
        .with_context(|| format!("move {} to {}", image.path.display(), out.display()))?;
    println!(
        "wrote {} ({} bytes), its SHA-256 the one the configuration names",
        out.display(),
        image.size
    );
    Ok(())
}

/// The size of the partition `stock` fills: all of it, as the phone and a
/// full update package hold it, which its AVB footer ends.
fn partition_size(stock: &Path) -> Result<usize> {
    let mut file = File::open(stock).with_context(|| format!("open {}", stock.display()))?;
    let size = file
        .metadata()
        .with_context(|| format!("stat {}", stock.display()))?
        .len();
    let mut magic = [0u8; 4];
    if size >= 64 {
        file.seek(SeekFrom::Start(size - 64))
            .with_context(|| format!("seek in {}", stock.display()))?;
        file.read_exact(&mut magic)
            .with_context(|| format!("read {}", stock.display()))?;
    }
    ensure!(
        &magic == b"AVBf",
        "{} does not end in an AVB footer; give the whole recovery partition, as the phone or a full update package holds it",
        stock.display()
    );
    Ok(usize::try_from(size)?)
}

/// The rollback index the stock image's vbmeta names, which the bootloader
/// keeps for the slot and compares with the vbmeta of every image it starts:
/// an image naming a smaller one is refused as a downgrade.
fn rollback(stock: &Path) -> Result<avb::Rollback> {
    let field = |bytes: &[u8]| u64::from_be_bytes(bytes.try_into().expect("eight bytes"));
    let mut file = File::open(stock).with_context(|| format!("open {}", stock.display()))?;
    let size = file
        .metadata()
        .with_context(|| format!("stat {}", stock.display()))?
        .len();
    ensure!(
        size >= 64,
        "{} is too short to hold an AVB footer",
        stock.display()
    );
    file.seek(SeekFrom::Start(size - 64))
        .with_context(|| format!("seek in {}", stock.display()))?;
    let mut footer = [0u8; 64];
    file.read_exact(&mut footer)
        .with_context(|| format!("read {}", stock.display()))?;
    ensure!(
        &footer[..4] == b"AVBf",
        "{} does not end in an AVB footer",
        stock.display()
    );
    let (offset, len) = (field(&footer[20..28]), field(&footer[28..36]));
    ensure!(
        len >= 256 && offset.checked_add(len).is_some_and(|end| end <= size - 64),
        "{} names its vbmeta image beyond its own end",
        stock.display()
    );
    file.seek(SeekFrom::Start(offset))
        .with_context(|| format!("seek in {}", stock.display()))?;
    let mut header = [0u8; 256];
    file.read_exact(&mut header)
        .with_context(|| format!("read {}", stock.display()))?;
    ensure!(
        &header[..4] == b"AVB0",
        "{} holds no vbmeta image where its footer points",
        stock.display()
    );
    let payload = field(&header[12..20])
        .checked_add(field(&header[20..28]))
        .and_then(|n| n.checked_add(256));
    ensure!(
        payload.is_some_and(|n| n <= len),
        "{} has truncated vbmeta blocks",
        stock.display()
    );
    Ok(avb::Rollback {
        index: field(&header[112..120]),
        location: u32::from_be_bytes(header[124..128].try_into().expect("four bytes")),
    })
}

/// Oplus starts a separate recovery UI and leaves the AOSP recovery
/// service in the default class. Start Skyward once instead, keeping all
/// original executables, libraries and policy in place.
fn prepare_oplus_init(rd: &mut Ramdisk, omit_ui: bool) -> Result<bool> {
    if rd.find("system/bin/phoenix_recovery").is_none() {
        ensure!(!omit_ui, "--omit-oplus-ui requires an Oplus stock recovery");
        return Ok(false);
    }
    let name = "system/etc/init/hw/init.rc";
    let init = rd
        .find(name)
        .with_context(|| format!("{name} is missing from the Oplus recovery"))?;
    let init = std::str::from_utf8(&init.data).context("Oplus init.rc is not UTF-8")?;
    let changed = oplus_init(init, omit_ui)?;
    rd.replace(name, changed.into_bytes())?;
    Ok(true)
}

fn oplus_init(init: &str, omit_ui: bool) -> Result<String> {
    let mut out = format!("import /{}\n", paths::MTP_RC);
    let (mut started, mut service) = (false, false);
    let mut disable = false;
    for line in init.lines() {
        let text = line.trim();
        if text == format!("import /{}", paths::MTP_RC) {
            continue;
        }
        if omit_ui && text.starts_with("exec ") && text.contains("/res/OplusOS/.") {
            continue;
        }
        if text == "start phoenix_recovery" {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(&format!("{indent}start recovery\n"));
            started = true;
            continue;
        }
        started |= text == "start recovery";
        if text == "service recovery /system/bin/recovery" {
            service = true;
            disable = true;
        } else if text == "service phoenix_recovery /system/bin/phoenix_recovery" {
            disable = true;
        } else if !line.starts_with(char::is_whitespace)
            && !text.is_empty()
            && !text.starts_with('#')
        {
            disable = false;
        }
        if disable && text == "disabled" {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if text == "service recovery /system/bin/recovery"
            || text == "service phoenix_recovery /system/bin/phoenix_recovery"
        {
            out.push_str("    disabled\n");
        }
    }
    ensure!(
        started && service,
        "the Oplus init script does not describe its expected recovery startup"
    );
    Ok(out)
}

/// Generic Oplus recovery properties name `ossi`, not the device checked
/// by its OTA packages. Give each property source the verified device id.
fn device_identity(props: &[u8], codename: &str) -> Vec<u8> {
    let names: Vec<String> = ["", "system.", "vendor.", "odm.", "product.", "system_ext."]
        .iter()
        .map(|part| format!("ro.product.{part}device"))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut out = without(props, &refs);
    out.push(b'\n');
    for name in names {
        out.extend_from_slice(format!("{name}={codename}\n").as_bytes());
    }
    out
}

/// What a recovery image says about the build it is: the version and patch
/// levels of its own ramdisk's system and vendor partition, which are older
/// than those installed beside it whenever the phone has taken an update.
/// The phone's secure services read these to tell the secure world which
/// build is running, and keys the installed system made are refused when
/// they say the wrong one, so they are left out here for the recovery to
/// fill in from the installed system once it has that mounted.
const BUILD_VERSION: [&str; 3] = [
    "ro.build.version.release",
    "ro.build.version.security_patch",
    "ro.vendor.build.security_patch",
];

/// The scheduling profiles a recovery's programs look up by name. A
/// recovery's cgroup setup mounts no controllers, so of all a system's
/// profiles do, only the timer slack applies here.
const TASK_PROFILES: &[u8] = include_bytes!("task_profiles.json");

/// The init rules that give the USB port file transfer beside adb, which a
/// stock recovery does not offer.
const MTP_RC: &[u8] = include_bytes!("mtp.rc");

/// A property file without the lines that set any of `names`.
fn without(props: &[u8], names: &[&str]) -> Vec<u8> {
    let mut out = Vec::with_capacity(props.len());
    for line in props.split_inclusive(|b| *b == b'\n') {
        let key = line.split(|b| *b == b'=').next().unwrap_or_default();
        if !names.iter().any(|n| n.as_bytes() == key) {
            out.extend_from_slice(line);
        }
    }
    out
}

/// Write `bytes`, the image `img` makes, to `out`.
fn write(out: &Path, img: &BootImage, bytes: &[u8]) -> Result<()> {
    std::fs::write(out, bytes).with_context(|| format!("write {}", out.display()))?;
    println!(
        "wrote {} ({} bytes kernel, {} bytes ramdisk, {} bytes in all)",
        out.display(),
        img.kernel.len(),
        img.ramdisk.len(),
        bytes.len()
    );
    Ok(())
}

fn list(image: &Path) -> Result<()> {
    let img = BootImage::read(image)?;
    let h = &img.header;
    println!(
        "header v{}  kernel {} bytes  ramdisk {} bytes  {:?}",
        h.header_version(),
        img.kernel.len(),
        img.ramdisk.len(),
        h.osversionpatch
    );
    if img.ramdisk.is_empty() {
        return Ok(());
    }
    let rd = Ramdisk::decode(&img.ramdisk)?;
    for e in &rd.entries {
        println!(
            "{:06o} {:>5}:{:<5} {:>10}  {}",
            e.mode,
            e.uid,
            e.gid,
            e.data.len(),
            e.name
        );
    }
    println!("{} entries", rd.entries.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_that_set_a_name_are_the_only_ones_left_out() {
        let props = b"ro.build.id=TQ\nro.build.version.release=13\nro.build.version.release_or_codename=13\nro.product.name=ishtar\n\
ro.vendor.build.security_patch=2026-10-01\nro.vendor.build.security_patch_level=1\n";
        let out = without(props, &BUILD_VERSION);
        let text = String::from_utf8(out).expect("property files are text");
        assert!(!text.contains("ro.build.version.release=13"));
        assert!(
            !text.contains("ro.vendor.build.security_patch=2026-10-01"),
            "the vendor partition's patch level is left out too"
        );
        assert!(
            text.contains("ro.build.version.release_or_codename=13"),
            "a longer name is left alone"
        );
        assert!(
            text.contains("ro.vendor.build.security_patch_level=1"),
            "a longer name is left alone"
        );
        assert!(text.contains("ro.build.id=TQ") && text.contains("ro.product.name=ishtar"));
    }

    #[test]
    fn the_profiles_name_all_the_recovery_asks_for() {
        let profiles: serde_json::Value =
            serde_json::from_slice(TASK_PROFILES).expect("the profiles are JSON");
        let names = |key: &str| -> Vec<String> {
            profiles[key]
                .as_array()
                .expect("a list")
                .iter()
                .map(|p| p["Name"].as_str().expect("a name").to_string())
                .collect()
        };
        let (plain, aggregate) = (names("Profiles"), names("AggregateProfiles"));
        // What libprocessgroup asks for when a program sets its scheduling
        // policy, and what the recovery's own init scripts give logd.
        let asked = [
            "SCHED_SP_DEFAULT",
            "SCHED_SP_BACKGROUND",
            "SCHED_SP_FOREGROUND",
            "SCHED_SP_TOP_APP",
            "SCHED_SP_SYSTEM",
            "SCHED_SP_RT_APP",
            "CPUSET_SP_DEFAULT",
            "CPUSET_SP_BACKGROUND",
            "CPUSET_SP_FOREGROUND",
            "CPUSET_SP_TOP_APP",
            "CPUSET_SP_SYSTEM",
            "CPUSET_SP_RESTRICTED",
            "BlkIOBackground",
            "BlkIOForeground",
            "ServiceCapacityLow",
        ];
        for name in asked {
            assert!(
                plain.iter().chain(&aggregate).any(|n| n == name),
                "{name} is missing"
            );
        }
        for a in profiles["AggregateProfiles"].as_array().expect("a list") {
            for part in a["Profiles"].as_array().expect("a list") {
                let part = part.as_str().expect("a name");
                assert!(
                    plain.iter().any(|n| n == part),
                    "{} is made of {part}, which is not a profile",
                    a["Name"]
                );
            }
        }
    }

    #[test]
    fn the_partition_is_as_large_as_a_stock_image_ending_in_its_footer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.img");
        let mut whole = vec![0u8; 8192];
        whole[8192 - 64..8192 - 60].copy_from_slice(b"AVBf");
        std::fs::write(&path, &whole).unwrap();
        assert_eq!(partition_size(&path).unwrap(), 8192);
        std::fs::write(&path, &whole[..8000]).unwrap();
        assert!(
            partition_size(&path).is_err(),
            "an image cut short of its partition's end"
        );
        std::fs::write(&path, b"AVBf").unwrap();
        assert!(
            partition_size(&path).is_err(),
            "too short to end in a footer"
        );
    }

    #[test]
    fn a_file_without_a_last_newline_keeps_its_last_line() {
        let out = without(b"a=1\nro.build.version.release=13", &BUILD_VERSION);
        assert_eq!(out, b"a=1\n");
        let kept = without(b"a=1\nb=2", &BUILD_VERSION);
        assert_eq!(kept, b"a=1\nb=2");
    }

    #[test]
    fn oplus_starts_skyward_once_and_imports_file_transfer() {
        let stock = "import /init.recovery.qcom.rc\non init\n    start servicemanager\n    start phoenix_recovery\n\non boot\n    exec u:r:recovery:s0 root root -- /system/bin/cp -af /res/OplusOS/. /res\n    class_start default\n\nservice phoenix_recovery /system/bin/phoenix_recovery\n    critical\n    user root\n\nservice recovery /system/bin/recovery\n    socket recovery stream 422 system system\n    user root\n    seclabel u:r:recovery:s0\n\nservice adbd /system/bin/adbd\n    disabled\n";
        let patched = oplus_init(stock, true).unwrap();
        assert!(patched.contains("    start recovery\n"));
        assert!(!patched.contains("    start phoenix_recovery\n"));
        assert!(
            patched
                .contains("service phoenix_recovery /system/bin/phoenix_recovery\n    disabled\n")
        );
        assert!(patched.contains("service recovery /system/bin/recovery\n    disabled\n"));
        assert!(patched.contains("    socket recovery stream 422 system system\n"));
        assert!(patched.contains("    seclabel u:r:recovery:s0\n"));
        assert!(patched.contains("import /init.recovery.qcom.rc\n"));
        assert_eq!(
            patched
                .matches("import /system/etc/init/skyward-mtp.rc\n")
                .count(),
            1
        );
        assert!(!patched.contains("/res/OplusOS/."));
        assert_eq!(oplus_init(&patched, true).unwrap(), patched);
        assert!(oplus_init(stock, false).unwrap().contains("/res/OplusOS/."));
        assert!(
            oplus_init("on init\n    start phoenix_recovery\n", false).is_err(),
            "do not silently build an unknown startup layout"
        );
    }

    #[test]
    fn oplus_package_identity_matches_every_property_source() {
        let props = b"ro.product.vendor.device=ossi\nro.product.odm.device=ossi\nro.product.device=wrong\nro.product.model=OPD2413\n";
        let out = String::from_utf8(device_identity(props, "OP615EL1")).unwrap();
        assert!(!out.contains("=ossi") && !out.contains("=wrong"));
        assert!(out.contains("ro.product.model=OPD2413\n"));
        for part in ["", "system.", "vendor.", "odm.", "product.", "system_ext."] {
            assert_eq!(
                out.matches(&format!("ro.product.{part}device=OP615EL1\n"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn stock_rollback_fields_are_read_and_malformed_offsets_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.img");
        let bytes = avb::footed(
            b"recovery",
            "recovery",
            16384,
            b"salt",
            avb::Rollback {
                index: 42,
                location: 7,
            },
        )
        .unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            rollback(&path).unwrap(),
            avb::Rollback {
                index: 42,
                location: 7
            }
        );
        let mut bad = bytes.clone();
        let offset = bad.len() - 44;
        bad[offset..offset + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        std::fs::write(&path, &bad).unwrap();
        assert!(
            rollback(&path).is_err(),
            "a wrapped offset is not a valid vbmeta location"
        );
        let mut bad = bytes;
        bad[4096..4100].copy_from_slice(b"nope");
        std::fs::write(&path, &bad).unwrap();
        assert!(
            rollback(&path).is_err(),
            "do not copy anti-rollback data from an invalid header"
        );
    }
}
