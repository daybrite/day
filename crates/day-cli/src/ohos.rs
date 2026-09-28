// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! HarmonyOS / OpenHarmony (`harmony-arkui`) pipeline: the OHOS analogue of mobile.rs's
//! android/iOS pipelines. `build_ohos` cross-compiles the app to `libentry.so`, then packages +
//! signs a `.hap` via the staged host under `<project>/build/day/harmony/project/`; `launch_ohos`
//! installs + starts it on a connected emulator/device over `hdc`.
//!
//! The emulator is an x86_64 OpenHarmony QEMU image: Oniro's (OpenHarmony 6.x), or
//! harmony-contrib/ohos-qemu's `x86_64_virt` (7.0), told apart by [`EmulatorImage`]. Either is a
//! networked hdc target (KVM-accelerated where `/dev/kvm` exists, i.e. x86_64 Linux CI, else
//! TCG), so every hdc call carries `-t <connect-key>` (default `127.0.0.1:55555`; override with
//! `DAY_OHOS_TARGET`). Building a `.hap` needs `hvigor` + `ohpm` on PATH (from the OpenHarmony
//! command-line-tools), the SDK via `OHOS_BASE_SDK_HOME` / `OHOS_NDK_HOME` (e.g. from
//! `openharmony-rs/setup-ohos-sdk`), and a JDK for signing. Two OHOS-only quirks the code
//! accounts for (see the CI research): `aa start` exits 0 even when the launch is refused (so we
//! parse its output for `Error Code:`), and `snapshot_display` writes JPEG (so the screenshot
//! path prefers `uitest screenCap`, which writes PNG). See docs/harmonyos.md.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

mod staging;

use crate::cli::Profile;
use crate::meta::Project;
use crate::mobile::{run_logged, rustup_cargo};
use crate::ops::{
    BuildOutcome, INSTALL_TIMEOUT, LAUNCH_TIMEOUT, LaunchSpec, LogStream, emit_log, status,
};
use crate::targets::Target;

/// The HarmonyOS host project's directory: `platform/harmony` (matching the target
/// identifier `harmony-arkui`, like every other platform dir; docs/harmonyos.md). An older
/// scaffold's `platform/ohos` still resolves, with a one-time rename hint; a project with
/// neither answers the modern path (scaffolding, error messages).
pub fn harmony_dir(project: &Project) -> PathBuf {
    let modern = project.root.join("platform/harmony");
    if modern.exists() {
        return modern;
    }
    let legacy = project.root.join("platform/ohos");
    if legacy.exists() {
        static HINTED: std::sync::Once = std::sync::Once::new();
        HINTED.call_once(|| {
            status(
                "Warning",
                "platform/ohos is the pre-rename layout — rename the directory to \
                 platform/harmony (day reads both for now)",
            );
        });
        return legacy;
    }
    modern
}

/// The writable hvigor project. Source stays in `harmony_dir`; generated manifests,
/// localized resources, native libraries and hvigor outputs stay under build/day.
/// Flavors get separate projects so their native metadata and resources cannot leak.
pub(crate) fn staged_harmony_dir(project: &Project) -> PathBuf {
    crate::ops::staged_root(project).join("harmony/project")
}

/// The user's home directory: `HOME`, or `USERPROFILE` on Windows, where `HOME` is usually unset.
pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The file names `name` may have on PATH. Windows adds `.exe`, `.bat`, and `.cmd`: the
/// OpenHarmony command-line tools ship `hvigorw.bat` and `ohpm.bat` there, and
/// `Command::new("hvigorw")` only ever tries `.exe`, so a tool is run by its resolved path.
pub(crate) fn tool_file_names(name: &str, windows: bool) -> Vec<String> {
    let mut names = vec![name.to_string()];
    if windows {
        names.extend([".exe", ".bat", ".cmd"].map(|ext| format!("{name}{ext}")));
    }
    names
}

/// Resolve a command-line tool on PATH the way the build runs it (see [`tool_file_names`]).
pub(crate) fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names = tool_file_names(name, cfg!(windows));
    std::env::split_paths(&path)
        .find_map(|dir| names.iter().map(|n| dir.join(n)).find(|p| p.is_file()))
}

/// `name` as an executable file name on this host (`hdc` → `hdc.exe` on Windows).
pub(crate) fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The x86_64 OpenHarmony emulator images `emulator_launch` boots, by the layout of their
/// directory (`DAY_OHOS_EMULATOR`). Both run under the same QEMU recipe; they differ in their
/// disks, kernel command line, and the port the guest's hdc listens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmulatorImage {
    /// Eclipse Oniro's `oniro_emulator.zip` (device_board_oniro; OpenHarmony 5.x and 6.x):
    /// four disks, hdc on guest port 55555.
    Oniro,
    /// harmony-contrib/ohos-qemu's `x86_64_virt` images (OpenHarmony 7.0): six disks (adding
    /// `sys_prod` and `chip_prod`), an eng/developer-mode boot, hdc on guest port 5555.
    OhosQemu,
}

impl EmulatorImage {
    /// The layout `dir` holds: ohos-qemu's when either of its extra partitions is present,
    /// else Oniro's. `Err` for ohos-qemu's arm64 package, which has an `Image` kernel rather
    /// than `bzImage` and needs `qemu-system-aarch64`.
    pub(crate) fn detect(dir: &Path) -> Result<EmulatorImage, String> {
        if dir.join("Image").is_file() && !dir.join("bzImage").is_file() {
            return Err(format!(
                "{} holds an arm64 image (an `Image` kernel); day boots the x86_64 emulator \
                 images only — use the x86_64_virt package",
                dir.display()
            ));
        }
        let ohos_qemu = ["sys_prod.img", "chip_prod.img"]
            .iter()
            .any(|f| dir.join(f).is_file());
        Ok(if ohos_qemu {
            EmulatorImage::OhosQemu
        } else {
            EmulatorImage::Oniro
        })
    }

    /// The files this layout boots from.
    pub(crate) fn files(self) -> &'static [&'static str] {
        match self {
            EmulatorImage::Oniro => &[
                "bzImage",
                "ramdisk.img",
                "updater.img",
                "system.img",
                "vendor.img",
                "userdata.img",
            ],
            EmulatorImage::OhosQemu => &[
                "bzImage",
                "ramdisk.img",
                "updater.img",
                "system.img",
                "vendor.img",
                "sys_prod.img",
                "chip_prod.img",
                "userdata.img",
            ],
        }
    }

    /// Which files of this layout `dir` lacks.
    pub(crate) fn missing(self, dir: &Path) -> Vec<&'static str> {
        self.files()
            .iter()
            .copied()
            .filter(|f| !dir.join(f).is_file())
            .collect()
    }

    /// A short name for status lines and doctor.
    pub(crate) fn label(self) -> &'static str {
        match self {
            EmulatorImage::Oniro => "Oniro layout",
            EmulatorImage::OhosQemu => "ohos-qemu layout, OpenHarmony 7.0",
        }
    }

    /// The port the guest's hdc daemon listens on; the host side stays `DAY_OHOS_TARGET`'s.
    fn guest_hdc_port(self) -> u16 {
        match self {
            EmulatorImage::Oniro => 55555,
            EmulatorImage::OhosQemu => 5555,
        }
    }

    /// The block devices, in the order the kernel names them (vda, vdb, …): the command line's
    /// `ohos.required_mount.*` entries refer to them by that name, so the order is load-bearing.
    fn disks(self) -> &'static [&'static str] {
        match self {
            EmulatorImage::Oniro => &["updater", "system", "vendor", "userdata"],
            EmulatorImage::OhosQemu => &[
                "updater",
                "system",
                "vendor",
                "sys_prod",
                "chip_prod",
                "userdata",
            ],
        }
    }

    /// The kernel command line, fixed per image build.
    fn append(self) -> &'static str {
        match self {
            EmulatorImage::Oniro => {
                "ip=dhcp loglevel=4 console=ttyS0,115200 init=init root=/dev/ram0 rw \
                 ohos.boot.hardware=x86_general \
                 ohos.required_mount.system=/dev/block/vdb@/usr@ext4@ro,barrier=1@wait,required \
                 ohos.required_mount.vendor=/dev/block/vdc@/vendor@ext4@ro,barrier=1@wait,required \
                 ohos.required_mount.misc=/dev/block/vda@/misc@none@none=@wait,required"
            }
            // From the release's launch/qemu_run.sh. `oemmode=rd buildvariant=eng
            // developer_mode=1` boots the developer device mode app installs rely on.
            EmulatorImage::OhosQemu => {
                "oemmode=rd buildvariant=eng developer_mode=1 console=ttyS0,115200 \
                 sn=0023456789 init=/bin/init hardware=virt root=/dev/ram0 rw ip=dhcp \
                 ohos.boot.hardware=virt \
                 ohos.required_mount.system=/dev/block/vdb@/usr@ext4@ro,barrier=1@wait,required \
                 ohos.required_mount.vendor=/dev/block/vdc@/vendor@ext4@ro,barrier=1@wait,required \
                 ohos.required_mount.sys_prod=/dev/block/vdd@/sys_prod@ext4@rw,barrier=1@wait,required \
                 ohos.required_mount.chip_prod=/dev/block/vde@/chip_prod@ext4@rw,barrier=1@wait,required \
                 ohos.required_mount.data=/dev/block/vdf@/data@f2fs@nosuid,nodev,noatime@wait,required,reservedsize=104857600"
            }
        }
    }

    /// The QEMU arguments this layout adds: its disks, and (ohos-qemu) the virtio tablet and
    /// keyboard its image is built to take input from. Oniro keeps its sound card.
    fn qemu_args(self) -> Vec<String> {
        let mut args = Vec::new();
        for (index, disk) in self.disks().iter().enumerate() {
            let drive = match self {
                EmulatorImage::Oniro => {
                    format!("if=none,file={disk}.img,format=raw,id={disk},index={index}")
                }
                EmulatorImage::OhosQemu => format!("if=none,file={disk}.img,format=raw,id={disk}"),
            };
            let device = match self {
                EmulatorImage::Oniro => format!("virtio-blk-pci,drive={disk}"),
                EmulatorImage::OhosQemu => format!("virtio-blk-pci,drive={disk},serial={disk}"),
            };
            args.extend(["-drive".into(), drive, "-device".into(), device]);
        }
        match self {
            EmulatorImage::Oniro => args.extend(["-device".into(), "es1370".into()]),
            EmulatorImage::OhosQemu => args.extend(
                [
                    "-device",
                    "virtio-tablet-pci",
                    "-device",
                    "virtio-keyboard-pci",
                ]
                .map(String::from),
            ),
        }
        args
    }
}

/// The emulator image directory: `DAY_OHOS_EMULATOR`, else `~/ohos/emulator/images`.
pub(crate) fn emulator_images_dir() -> PathBuf {
    std::env::var_os("DAY_OHOS_EMULATOR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("ohos").join("emulator").join("images"))
}

/// The API levels a versioned SDK root (`OHOS_BASE_SDK_HOME`) holds: numeric subdirectories with
/// the SDK's `ets` or `toolchains` component, ascending. hvigor's OpenHarmony mode reads the one
/// named by the project's `compileSdkVersion`, and refuses an unversioned root ("The SDK
/// management mode has changed").
pub(crate) fn sdk_api_levels(base: &Path) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut levels: Vec<u32> = entries
        .flatten()
        .filter_map(|e| {
            let level: u32 = e.file_name().to_str()?.parse().ok()?;
            let dir = e.path();
            (dir.join("ets").is_dir() || dir.join("toolchains").is_dir()).then_some(level)
        })
        .collect();
    levels.sort_unstable();
    levels
}

/// The executable format a file starts with, to catch an SDK for another OS: hvigor spawns the
/// SDK's tools directly, so a Linux SDK on macOS fails deep in the build with `spawn ENOEXEC`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryFormat {
    Elf,
    MachO,
    Pe,
}

impl BinaryFormat {
    /// What `host` (`linux`/`macos`/`windows`, as `targets::host_os` answers) runs natively.
    pub(crate) fn for_host(host: &str) -> Option<BinaryFormat> {
        match host {
            "linux" => Some(BinaryFormat::Elf),
            "macos" => Some(BinaryFormat::MachO),
            "windows" => Some(BinaryFormat::Pe),
            _ => None,
        }
    }

    /// The OS this format belongs to, for the error message.
    pub(crate) fn os(self) -> &'static str {
        match self {
            BinaryFormat::Elf => "Linux",
            BinaryFormat::MachO => "macOS",
            BinaryFormat::Pe => "Windows",
        }
    }

    /// Classify by magic number. `0xCAFEBABE` (Mach-O fat) is also Java's class magic, but no SDK
    /// tool this is asked about is a class file.
    pub(crate) fn detect(head: &[u8]) -> Option<BinaryFormat> {
        match head {
            [0x7f, b'E', b'L', b'F', ..] => Some(BinaryFormat::Elf),
            [b'M', b'Z', ..] => Some(BinaryFormat::Pe),
            [0xfe, 0xed, 0xfa, 0xce | 0xcf, ..]
            | [0xce | 0xcf, 0xfa, 0xed, 0xfe, ..]
            | [0xca, 0xfe, 0xba, 0xbe, ..] => Some(BinaryFormat::MachO),
            _ => None,
        }
    }

    /// The format of the file at `path` (its first four bytes), `None` if unreadable or unknown.
    pub(crate) fn of_file(path: &Path) -> Option<BinaryFormat> {
        use std::io::Read;
        let mut head = [0u8; 4];
        std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
        BinaryFormat::detect(&head)
    }
}

/// The emulator's default vCPU count: 6, or the host's core count when it has fewer.
fn default_smp() -> usize {
    std::thread::available_parallelism().map_or(6, |n| n.get().min(6))
}

/// The size QEMU's GTK display opens its window at, before the guest has set a scanout; a
/// windowed Linux guest runs at this size whatever panel was asked for (see `emulator_launch`).
const GTK_WINDOW_PANEL: (u32, u32) = (640, 480);

/// Bring up the OpenHarmony QEMU emulator as a native window (the OHOS analogue of
/// `skip android emulator launch`). On macOS the QEMU `cocoa` backend opens a native window
/// directly, with no VNC or Screen Sharing in between, and on Linux the `gtk` one;
/// `--headless` uses no display (hdc-only, for CI). Self-contained: it builds the QEMU command
/// itself, so it doesn't depend on the emulator distribution's shell launcher.
///
/// The image directory is `DAY_OHOS_EMULATOR` or the default `~/ohos/emulator/images`, holding
/// either an Oniro image (OpenHarmony 6.x) or an ohos-qemu `x86_64_virt` one (7.0); its files
/// decide which ([`EmulatorImage`]), and with it the disks, kernel command line, and guest hdc
/// port. The host hdc port comes from `DAY_OHOS_TARGET` (default `127.0.0.1:55555`) for both, so
/// every later `hdc` call, `day launch` included, addresses either image the same way.
///
/// `panel` is the guest display in pixels: the image has no screen of its own, it draws at
/// whatever the virtio-gpu is told, which is how one image serves as a phone (360×720) and as a
/// landscape tablet (1280×800). The size is exported as `DAY_OHOS_PANEL` (through `GITHUB_ENV`
/// on a runner) so the runs that follow know where the keyguard swipe lands.
pub fn emulator_launch(headless: bool, panel: (u32, u32)) -> Result<(), String> {
    let images = emulator_images_dir();
    let image = EmulatorImage::detect(&images)?;
    let missing = image.missing(&images);
    if !missing.is_empty() {
        return Err(format!(
            "OpenHarmony emulator images not found at {} (missing {}). Download the Oniro \
             emulator (OpenHarmony 6.x) or an ohos-qemu x86_64_virt package (7.0) and set \
             DAY_OHOS_EMULATOR to its image dir (see docs/harmonyos.md).",
            images.display(),
            missing.join(", ")
        ));
    }
    let qemu = "qemu-system-x86_64";
    if Command::new(qemu).arg("--version").output().is_err() {
        return Err(format!(
            "{qemu} not found — install QEMU to run the OpenHarmony emulator \
             (`day doctor --toolkit harmonyos` shows how on this host)."
        ));
    }
    // Host hdc port from the connect key (the guest's own port is the image's). Kill any stale hdc
    // server first so it can't hold the host port before QEMU binds the forward.
    let _ = Command::new(hdc_bin()).arg("kill").output();
    // The requested port is often already occupied (GitHub's macOS runners hold 55555, and so
    // do some local services), and QEMU then dies instantly ("Could not set up host forwarding
    // rule"), leaving no reachable target. Probe and slide to the first free port; the chosen
    // key is tconn'ed below (so `hdc list targets` discovery finds it) and exported through
    // GITHUB_ENV so later CI steps target it too.
    let requested: u16 = ohos_target()
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(55555);
    let host_port = (requested..requested.saturating_add(16))
        .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
        .ok_or_else(|| {
            format!(
                "no free hdc forward port near {requested} (tried {requested}..={})",
                requested.saturating_add(15)
            )
        })?;
    let target = format!("127.0.0.1:{host_port}");
    if host_port != requested {
        status(
            "Emulator",
            &format!(
                "port {requested} is in use — forwarding hdc on {target} instead \
                 (export DAY_OHOS_TARGET={target} for other shells)"
            ),
        );
        if let Ok(github_env) = std::env::var("GITHUB_ENV") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(github_env) {
                let _ = writeln!(f, "DAY_OHOS_TARGET={target}");
            }
        }
    }

    // The display backend: a native window locally (cocoa on macOS), none when headless.
    let display: &[&str] = if headless {
        &["-display", "none"]
    } else if cfg!(target_os = "macos") {
        // Plain cocoa: launching with zoom-to-fit=on stalls the guest's display bring-up
        // (bootevent.wms.fullscreen.ready never fires; three consecutive boots). To enlarge
        // the window, toggle View → Zoom To Fit once booted and drag-resize.
        &["-display", "cocoa"]
    } else if image == EmulatorImage::OhosQemu {
        // The ohos-qemu (7.0) guest aborts QEMU's GL display during boot
        // (`surface_gl_create_texture: Assertion 'map_format(...)' failed`, Homebrew QEMU 11.1,
        // Ubuntu 24.04), though its settled scanout is the same XRGB8888 as Oniro's. `gl=off` is
        // what the image's own launcher uses; where plain GTK doesn't repaint (below), boot
        // with --headless instead.
        &["-display", "gtk,gl=off"]
    } else {
        // GTK with OpenGL rendering. Plain `gtk` (cairo) painted its "Display output is not
        // active" placeholder once and never repainted (a Homebrew QEMU 11.1 on Ubuntu 24.04,
        // X11 and Wayland alike), even though the guest was scanning out and `screendump` saw
        // its frames; `gl=on` shows them.
        &["-display", "gtk,gl=on"]
    };

    // Kernel command line, disks, and the guest's hdc port come from the image's layout.
    let hostfwd = format!(
        "user,id=net0,hostfwd=tcp:127.0.0.1:{host_port}-:{}",
        image.guest_hdc_port()
    );
    // QEMU's GTK window opens at its 640×480 placeholder size and reports that size to
    // virtio-gpu, and the guest adopts it over the requested panel. Any other panel makes the
    // guest switch modes while its display is coming up; on a fast (KVM) boot that switch's
    // atomic commit fails with ENOSPC, the CRTC is never enabled, and the window shows
    // "Display output is not active" for good (restarting render_service doesn't recover it).
    // So a windowed Linux boot asks for 640×480 from the start. Later window resizes are safe:
    // the guest keeps 640×480. `--headless` honors the requested panel.
    let (xres, yres) = if !headless && !cfg!(target_os = "macos") {
        if panel != GTK_WINDOW_PANEL {
            status(
                "Panel",
                &format!(
                    "{}×{} requested, but a QEMU GTK window runs the guest at 640×480; \
                     boot with --headless to keep the requested panel",
                    panel.0, panel.1
                ),
            );
        }
        GTK_WINDOW_PANEL
    } else {
        panel
    };
    status("Panel", &format!("{xres}×{yres} (virtio-gpu)"));
    if let Ok(github_env) = std::env::var("GITHUB_ENV") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(github_env) {
            let _ = writeln!(f, "DAY_OHOS_PANEL={xres}x{yres}");
        }
    }
    let gpu = format!("virtio-gpu-pci,xres={xres},yres={yres},max_outputs=1,addr=08.0");
    // vCPU count (DAY_OHOS_SMP, default 6, capped at the host's cores). On a busy host fewer
    // vCPUs boot more reliably: TCG vCPU threads that lose the CPU while holding a guest
    // spinlock leave the other vCPUs spinning (guest load explodes, WMS/boot services stall),
    // classic lock-holder preemption, and more vCPUs than cores guarantees that contention.
    let smp = std::env::var("DAY_OHOS_SMP").unwrap_or_else(|_| default_smp().to_string());

    // Accelerator: the Oniro guest is x86_64, so on a same-arch host that exposes `/dev/kvm`
    // (an x86_64 Linux CI runner with nested virtualization) it runs KVM-accelerated at
    // near-native speed instead of TCG software emulation, cutting the boot + walkthrough from
    // ~tens of minutes to minutes. macOS/dev hosts have no `/dev/kvm`, so they stay on TCG.
    // A Linux desktop often has `/dev/kvm` owned by the `kvm` group without the user in it;
    // qemu then dies with "Could not access KVM kernel module: Permission denied", so KVM is
    // chosen only when the device opens read-write, with a hint otherwise.
    // `DAY_OHOS_ACCEL` overrides (e.g. `tcg,thread=multi` to force software, or `kvm`).
    // `-cpu host` (full passthrough) pairs with KVM; TCG needs the emulated `-cpu max`.
    let kvm = std::path::Path::new("/dev/kvm");
    let (accel, cpu) = match std::env::var("DAY_OHOS_ACCEL") {
        Ok(a) if !a.is_empty() => {
            let cpu = if a.starts_with("kvm") { "host" } else { "max" };
            (a, cpu)
        }
        _ if std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(kvm)
            .is_ok() =>
        {
            ("kvm".to_string(), "host")
        }
        _ => {
            if kvm.exists() {
                status(
                    "Emulator",
                    "/dev/kvm is not accessible to this user, so falling back to slow TCG; \
                     `sudo usermod -aG kvm $USER` and log in again to enable KVM",
                );
            }
            ("tcg,thread=multi".to_string(), "max")
        }
    };

    let mut cmd = Command::new(qemu);
    cmd.current_dir(&images)
        .args([
            "-machine", "q35", "-smp", &smp, "-m", "4096M", "-boot", "c", "-vga", "none",
        ])
        .args(["-device", &gpu])
        .args(display)
        .args(["-rtc", "base=utc,clock=host"])
        .args(["-initrd", "ramdisk.img", "-kernel", "bzImage"])
        .args(image.qemu_args())
        .args(["-serial", "none", "-append", image.append()])
        .args(["-accel", &accel, "-cpu", cpu])
        .args(["-netdev", &hostfwd, "-device", "virtio-net-pci,netdev=net0"]);
    status(
        "Emulator",
        &format!(
            "OpenHarmony ({}, {}) — {}",
            images.display(),
            image.label(),
            if headless { "headless" } else { "windowed" }
        ),
    );
    let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
    crate::signals::register_child(child.id());

    // Wait for hdc to see the target booted (TCG boot is slow), like `skip android emulator launch`.
    status(
        "Emulator",
        if accel == "kvm" {
            "waiting for boot (KVM-accelerated)…"
        } else {
            "waiting for boot (TCG software emulation is slow — up to ~8 min)…"
        },
    );
    for _ in 0..96 {
        if let Some(code) = child.try_wait().ok().flatten() {
            return Err(format!("qemu exited early ({code})"));
        }
        let _ = Command::new(hdc_bin()).args(["tconn", &target]).output();
        let booted = Command::new(hdc_bin())
            .args([
                "-t",
                &target,
                "shell",
                "param",
                "get",
                "bootevent.boot.completed",
            ])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
            .unwrap_or(false);
        if booted {
            status("Emulator", &format!("booted — hdc target {target}"));
            // Which OpenHarmony came up, so a log says whether a run was 6.x or 7.0.
            let param = |name: &str| {
                Command::new(hdc_bin())
                    .args(["-t", &target, "shell", "param", "get", name])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|v| !v.is_empty() && !v.contains("fail"))
            };
            if let Some(name) = param("const.ohos.fullname") {
                let api = param("const.ohos.apiversion")
                    .map(|a| format!(" (API {a})"))
                    .unwrap_or_default();
                status("Emulator", &format!("{name}{api}"));
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    Err("emulator did not report boot within the timeout (still starting?)".into())
}

/// The hdc target key (`-t`) for the emulator/device. Oniro's QEMU emulator is a networked target
/// reachable at the emulator-action connect-key `127.0.0.1:55555`; override via `DAY_OHOS_TARGET`
/// (a real device's connect key, or a different port).
pub fn ohos_target() -> String {
    std::env::var("DAY_OHOS_TARGET").unwrap_or_else(|_| "127.0.0.1:55555".into())
}

/// The `hdc` executable: on PATH if present, else resolved from the SDK install's sibling
/// `toolchains/` dir (the public SDK ships it there, next to the `native` NDK), so
/// `day launch -p harmony-arkui` works from GUI-launched editors whose environment has neither the
/// variable nor the PATH entry.
fn hdc_bin() -> &'static str {
    static HDC: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HDC.get_or_init(|| {
        if find_tool("hdc").is_some() {
            return "hdc".into();
        }
        if let Ok(ndk) = find_ohos_ndk() {
            let cand = Path::new(&ndk)
                .parent()
                .map(|p| p.join("toolchains").join(exe_name("hdc")));
            if let Some(c) = cand
                && c.is_file()
            {
                return c.to_string_lossy().into_owned();
            }
        }
        "hdc".into()
    })
}

/// Whether `hdc` can be run, so a listing can say "not installed" rather than "nothing
/// connected", two very different answers for someone wondering where their device went.
pub(crate) fn hdc_available() -> bool {
    Command::new(hdc_bin())
        .arg("-v")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A fresh `hdc` command targeting the default connect key (`DAY_OHOS_TARGET`).
pub fn hdc() -> Command {
    hdc_for(&ohos_target())
}

/// Forward host `tcp:port` → the app's dayscript engine on the launched target (hdc's
/// `adb forward`). Pinned to the device this run launched on, else the first discovered one: the
/// emulator's connect key may have auto-slid off an occupied default port (see
/// [`emulator_launch`]), so the env/default key can be stale. The forward intermittently fails
/// with "[Fail]TCP Port listen failed" when the host-side hdc server is in a bad state, so recycle
/// the server and retry (bounded); a recycled server has forgotten networked targets, so
/// re-`tconn` before every attempt (harmless for USB keys, which are auto-discovered).
pub(crate) fn fport_engine(port: u16) {
    let key = ohos_devices()
        .first()
        .map(|d| d.key.clone())
        .unwrap_or_else(ohos_target);
    for attempt in 1..=5u32 {
        let _ = Command::new(hdc_bin()).args(["tconn", &key]).output();
        let out = hdc_for(&key)
            .args(["fport", &format!("tcp:{port}"), &format!("tcp:{port}")])
            .output();
        let text = out
            .map(|o| {
                String::from_utf8_lossy(&o.stdout).into_owned()
                    + &String::from_utf8_lossy(&o.stderr)
            })
            .unwrap_or_default();
        if !text.contains("[Fail]") {
            return;
        }
        eprintln!(
            "day: hdc fport failed (attempt {attempt}/5): {} — retrying",
            text.trim()
        );
        let _ = Command::new(hdc_bin()).arg("kill").status();
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// A fresh `hdc` command pinned to connect key `key` (`-t <key>`), for multi-device install/launch.
fn hdc_for(key: &str) -> Command {
    let mut c = Command::new(hdc_bin());
    c.args(["-t", key]);
    c
}

/// A connected OpenHarmony target: its `hdc` connect key + the arch it runs (queried via
/// `uname -m`, mapped to the Rust triple + hap ABI dir). An emulator is x86_64; a device is
/// arm64; the query tells them apart.
pub(crate) struct OhosDevice {
    pub key: String,
    pub triple: &'static str,
    pub abi: &'static str,
}

/// arch string from `uname -m` → (Rust triple, hap ABI dir).
fn arch_triple(uname: &str) -> Option<(&'static str, &'static str)> {
    match uname.trim() {
        "aarch64" | "arm64" => Some(("aarch64-unknown-linux-ohos", "arm64-v8a")),
        "x86_64" | "amd64" => Some(("x86_64-unknown-linux-ohos", "x86_64")),
        _ => None,
    }
}

/// Connected OHOS targets. `hdc list targets` lists USB/attached keys; the networked emulator is
/// reached via `DAY_OHOS_TARGET`, so that key is always included (after a best-effort `tconn`). Each
/// target's arch is queried with `uname -m`. Unreachable targets are dropped.
pub(crate) fn ohos_devices() -> Vec<OhosDevice> {
    let mut keys: Vec<String> = Vec::new();
    // The default/networked target: connect + include it.
    let default_key = ohos_target();
    let _ = Command::new(hdc_bin())
        .args(["tconn", &default_key])
        .output();
    keys.push(default_key);
    // Any additional attached targets.
    if let Ok(out) = Command::new(hdc_bin()).args(["list", "targets"]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let k = line.trim();
            if !k.is_empty() && !k.starts_with('[') && !keys.iter().any(|e| e == k) {
                keys.push(k.to_string());
            }
        }
    }
    // Narrowed to what this run selected (`--ohos-device`), when it named one. Doing it here
    // rather than at each call site is what also pins `fport_engine`'s first-device pick, which
    // otherwise forwarded the dayscript port to whichever target answered first.
    if let Some(want) = crate::ops::selected_ohos_key() {
        keys.retain(|k| k == want);
    }
    keys.into_iter()
        .filter_map(|key| {
            let uname = Command::new(hdc_bin())
                .args(["-t", &key, "shell", "uname", "-m"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let (triple, abi) = arch_triple(&uname)?;
            Some(OhosDevice { key, triple, abi })
        })
        .collect()
}

/// Map a `DAY_OHOS_ARCH` value to its (triple, abi).
fn ohos_arch_override(v: &str) -> (&'static str, &'static str) {
    match v {
        "device" | "arm64" | "arm64-v8a" => ("aarch64-unknown-linux-ohos", "arm64-v8a"),
        _ => ("x86_64-unknown-linux-ohos", "x86_64"),
    }
}

/// The (triple, abi) set to build for: an explicit `DAY_OHOS_ARCH`, else the distinct arches of the
/// connected targets, else the emulator default so `day build` still produces a hap.
///
/// The override is checked first, and that ordering matters. Probing devices first meant a
/// distribution `day pack` changed shape depending on what happened to be plugged in: CI packs
/// with `DAY_OHOS_ARCH=arm64` but boots an x86_64 emulator for the walkthrough first, so the hap
/// shipped x86_64 and the same commit packed elsewhere shipped arm64. A pack must not be steered
/// by an attached device (§20.3). Dev flows are unaffected: they leave the variable unset and
/// still get every connected target's arch.
/// Just the ABI names of `ohos_build_arches`, for the provenance record.
pub(crate) fn build_abis() -> Vec<String> {
    ohos_build_arches()
        .into_iter()
        .map(|(_, abi)| abi.to_string())
        .collect()
}

pub(crate) fn ohos_build_arches() -> Vec<(&'static str, &'static str)> {
    if let Ok(v) = std::env::var("DAY_OHOS_ARCH")
        && !v.is_empty()
    {
        return vec![ohos_arch_override(&v)];
    }
    let mut arches: Vec<(&'static str, &'static str)> = ohos_devices()
        .into_iter()
        .map(|d| (d.triple, d.abi))
        .collect();
    arches.sort();
    arches.dedup();
    if arches.is_empty() {
        arches.push(("x86_64-unknown-linux-ohos", "x86_64"));
    }
    arches
}

/// The OpenHarmony NDK (`native` dir) for the cross-linker: `OHOS_NDK_HOME` (set by CI's
/// setup-ohos-sdk) if present, else a couple of common local install paths (see docs/harmonyos.md:
/// extract the public SDK's `native` component). Validated by the presence of `llvm/bin`.
pub(crate) fn find_ohos_ndk() -> Result<String, String> {
    if let Ok(v) = std::env::var("OHOS_NDK_HOME") {
        return Ok(v);
    }
    let home = home_dir();
    for cand in [
        home.join("ohos").join("ndk-extract").join("native"),
        home.join("ohos-sdk").join("native"),
    ] {
        if cand.join("llvm").join("bin").is_dir() {
            return Ok(cand.to_string_lossy().into_owned());
        }
    }
    Err(
        "OHOS_NDK_HOME is not set and no OpenHarmony NDK was found — set it to the SDK's `native` \
         directory (see docs/harmonyos.md)"
            .into(),
    )
}

/// Keep the two HarmonyOS files that spell out the app's identity in step with Day.toml: the
/// bundle id in `AppScope/app.json5`, and the deep-link scheme in the ability's `uris` skill in
/// `module.json5` (docs/deep-links.md).
///
/// iOS reads its identity through a generated xcconfig and Android through a generated
/// properties file; OHOS reads the merged manifests in the staged hvigor project. The source
/// manifests are never edited. Only the two identity fields change in the copies.
fn sync_ohos_identity(project: &Project) -> Result<(), String> {
    let resolved = project.manifest.resolve("harmony-arkui");
    let dir = staged_harmony_dir(project);

    let app_json = dir.join("AppScope/app.json5");
    if app_json.exists() {
        let text = std::fs::read_to_string(&app_json)
            .map_err(|e| format!("{}: {e}", app_json.display()))?;
        let out = replace_json5_string(&text, "bundleName", &resolved.id)?;
        if out != text {
            std::fs::write(&app_json, out).map_err(|e| format!("{}: {e}", app_json.display()))?;
        }
    }

    let module = dir.join("entry/src/main/module.json5");
    if module.exists() {
        let text =
            std::fs::read_to_string(&module).map_err(|e| format!("{}: {e}", module.display()))?;
        let out = replace_json5_string(&text, "scheme", &resolved.scheme())?;
        if out != text {
            std::fs::write(&module, out).map_err(|e| format!("{}: {e}", module.display()))?;
        }
    }
    Ok(())
}

/// Update string properties through the round-trip AST, preserving all unrelated source text.
fn replace_json5_string(text: &str, key: &str, value: &str) -> Result<String, String> {
    use crate::json5::{self, Value};
    fn visit(node: &mut Value, key: &str, value: &str) -> Result<(), String> {
        match node {
            Value::JSONObject {
                key_value_pairs, ..
            } => {
                for pair in key_value_pairs {
                    if json5::string(&pair.key).as_deref() == Some(key)
                        && json5::string(&pair.value).is_some()
                    {
                        json5::set_string(&mut pair.value, value)?;
                    } else {
                        visit(&mut pair.value, key, value)?;
                    }
                }
            }
            Value::JSONArray { values, .. } => {
                for item in values {
                    visit(&mut item.value, key, value)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut doc = json5::parse(text)?;
    visit(&mut doc.value, key, value)?;
    Ok(doc.to_string())
}

fn entry_ability(module: &crate::json5::Value) -> Option<&crate::json5::Value> {
    use crate::json5::{array, get, string};
    array(get(module, "abilities")?)
        .find(|a| get(a, "name").and_then(string).as_deref() == Some("EntryAbility"))
}

/// Read the owning ability's skill URI and merge its metadata without disturbing other entries.
fn shortcut_module(text: &str) -> Result<(String, Option<String>, String), String> {
    use crate::json5::{self, Value, array, get, get_mut, string};
    let mut doc = json5::parse(text)?;
    let module = get_mut(&mut doc.value, "module").ok_or("module.json5 has no module object")?;
    let module_name = get(module, "name")
        .and_then(string)
        .ok_or("module has no name")?;
    let ability = entry_ability(module).ok_or("no EntryAbility to attach [[shortcuts]] to")?;
    let scheme = get(ability, "skills")
        .into_iter()
        .flat_map(array)
        .filter_map(|skill| get(skill, "uris"))
        .flat_map(array)
        .find_map(|uri| get(uri, "scheme").and_then(string));
    let Value::JSONArray { values, .. } = get_mut(module, "abilities").unwrap() else {
        unreachable!()
    };
    let ability = &mut values
        .iter_mut()
        .find(|a| get(&a.value, "name").and_then(string).as_deref() == Some("EntryAbility"))
        .unwrap()
        .value;
    if get(ability, "metadata").is_none() {
        json5::insert(ability, "metadata", json5::parse("[]")?.value)?;
    }
    let metadata = get_mut(ability, "metadata").unwrap();
    if !matches!(metadata, Value::JSONArray { .. }) {
        return Err("EntryAbility.metadata must be an array".into());
    }
    let existing = match metadata {
        Value::JSONArray { values, .. } => values.iter_mut().find(|v| {
            get(&v.value, "name").and_then(string).as_deref() == Some("ohos.ability.shortcuts")
        }),
        _ => unreachable!(),
    };
    if let Some(existing) = existing {
        if let Some(resource) = get_mut(&mut existing.value, "resource") {
            json5::set_string(resource, "$profile:shortcuts_config")?;
        } else {
            json5::insert(
                &mut existing.value,
                "resource",
                json5::parse("'$profile:shortcuts_config'")?.value,
            )?;
        }
    } else {
        json5::push(
            metadata,
            json5::parse(
                r#"{ "name": "ohos.ability.shortcuts", "resource": "$profile:shortcuts_config" }"#,
            )?
            .value,
        )?;
    }
    Ok((module_name, scheme, doc.to_string()))
}

/// Write the declared permissions into `module.json5`, and their reasons into the module's string
/// resources.
///
/// HarmonyOS requires a user_grant permission's `reason` to be a `$string:` resource reference,
/// not literal text, so the two files are written together. Both writers are idempotent and touch
/// only what Day owns: a marker region in `module.json5`, and the `day_perm_reason_` prefix in
/// `string.json`.
fn sync_ohos_permissions(project: &Project) -> Result<(), String> {
    let module = staged_harmony_dir(project).join("entry/src/main/module.json5");
    if !module.exists() {
        return Ok(());
    }
    let contributed = crate::pieces::contributed_permissions(project, &["arkui"]);
    let plan = crate::permissions::resolve_project(project, "ohos", &contributed)
        .map_err(|e| format!("Day.toml: {e}"))?;
    let entries = crate::permissions::ohos_entries(&plan);
    for e in &entries {
        if e.name.ends_with("READ_IMAGEVIDEO") {
            status("Packing", day_build::permissions::OHOS_PHOTOS_APL_NOTE);
        }
    }

    // The ability the permissions are used by; the scaffold has exactly one. Omitting
    // `abilities` is safer than naming one that doesn't exist, which hvigor rejects.
    let before =
        std::fs::read_to_string(&module).map_err(|e| format!("{}: {e}", module.display()))?;
    let doc = crate::json5::parse(&before)?;
    let ability = crate::json5::get(&doc.value, "module")
        .and_then(entry_ability)
        .map(|_| "EntryAbility");

    let mut body = String::new();
    for e in &entries {
        body.push_str("      { \"name\": \"");
        body.push_str(&e.name);
        body.push('"');
        if let Some(key) = &e.reason_key {
            body.push_str(&format!(", \"reason\": \"$string:{key}\""));
        }
        if let Some(ability) = ability {
            body.push_str(&format!(
                ", \"usedScene\": {{ \"abilities\": [\"{ability}\"], \"when\": \"{}\" }}",
                e.when
            ));
        }
        body.push_str(" },\n");
    }

    // Don't add an empty region. An existing region is still emptied when the last permission
    // is removed, including one copied from a host prepared by an older CLI.
    if entries.is_empty() && !crate::json5::has_region(&before, "permissions")? {
        return write_ohos_reason_strings(project, &plan);
    }
    let with_region = crate::json5::ensure_region(&before, "requestPermissions", "permissions")?;
    let after = crate::json5::replace_region(&with_region, "permissions", &body)
        .ok_or_else(|| format!("{}: could not place the managed region", module.display()))?;
    if after != before {
        std::fs::write(&module, after).map_err(|e| format!("{}: {e}", module.display()))?;
    }

    write_ohos_reason_strings(project, &plan)
}

/// Merge the generated `day_perm_reason_*` entries into the module's `string.json`s, preserving
/// every other entry in its existing order: the default locale into `base/`, and every locale
/// the catalogs translate into its own qualifier directory (`zh_CN/`, `fr/`: the tag with the
/// hyphen HarmonyOS does not allow replaced), created when missing.
fn write_ohos_reason_strings(
    project: &Project,
    plan: &crate::permissions::Plan,
) -> Result<(), String> {
    let resources = staged_harmony_dir(project).join("entry/src/main/resources");
    let base = resources.join("base/element/string.json");
    if !base.exists() {
        return Ok(());
    }
    clear_day_strings(&resources, "day_perm_reason_")?;
    merge_day_strings(
        &base,
        "day_perm_reason_",
        &crate::permissions::ohos_reason_strings(plan),
    )?;
    for (locale, reasons) in crate::permissions::ohos_reason_strings_localized(plan) {
        if locale == plan.default_locale {
            continue;
        }
        let path = resources
            .join(locale.replace('-', "_"))
            .join("element/string.json");
        merge_day_strings(&path, "day_perm_reason_", &reasons)?;
    }
    Ok(())
}

/// Clear old managed translations, including locales no longer declared by the app. The
/// staged project may have copied these from a host generated by a previous CLI version.
fn clear_day_strings(resources: &Path, prefix: &str) -> Result<(), String> {
    if !resources.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(resources).map_err(|e| e.to_string())? {
        let strings = entry
            .map_err(|e| e.to_string())?
            .path()
            .join("element/string.json");
        if strings.exists() {
            merge_day_strings(&strings, prefix, &std::collections::BTreeMap::new())?;
        }
    }
    Ok(())
}

/// Merge day-owned entries into a `string.json`, preserving every other entry in its existing
/// order. The `prefix` IS the ownership marker: an entry whose source declaration was removed
/// disappears with no state file to consult. Creates the file (scaffold layout) when it doesn't
/// exist and there is something to write.
fn merge_day_strings(
    path: &std::path::Path,
    prefix: &str,
    entries: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let before = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) if entries.is_empty() => return Ok(()),
        Err(_) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            "{ \"string\": [\n] }\n".to_string()
        }
    };
    let doc: serde_json::Value =
        serde_json::from_str(&before).map_err(|e| format!("{}: {e}", path.display()))?;
    let existing = doc
        .get("string")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("{}: no \"string\" array", path.display()))?;

    let mut kept: Vec<(String, String)> = Vec::new();
    for item in existing {
        let (Some(name), Some(value)) = (
            item.get("name").and_then(|v| v.as_str()),
            item.get("value").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if !name.starts_with(prefix) {
            kept.push((name.to_string(), value.to_string()));
        }
    }
    for (k, v) in entries {
        kept.push((k.clone(), v.clone()));
    }

    // Hand-rolled to match the scaffold's exact layout; `to_string_pretty` uses a different one,
    // which would rewrite the whole file on the first build.
    let mut out = String::from("{ \"string\": [\n");
    for (i, (name, value)) in kept.iter().enumerate() {
        let comma = if i + 1 == kept.len() { "" } else { "," };
        out.push_str(&format!(
            "  {{ \"name\": {}, \"value\": {} }}{comma}\n",
            serde_json::to_string(name).unwrap_or_default(),
            serde_json::to_string(value).unwrap_or_default()
        ));
    }
    out.push_str("] }\n");
    if out != before {
        std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Day.toml `[[shortcuts]]` → the module's launcher-shortcut declaration: the
/// `$profile:shortcuts_config` JSON, an `ohos.ability.shortcuts` metadata entry on the main
/// ability, and `day_shortcut_*` label strings merged into each locale's `string.json`
/// (docs/deep-links.md "Shortcuts are saved deep links"). Each shortcut's want carries the
/// deep link in `parameters["day.uri"]`; EntryAbility forwards it through the same `deepLink`
/// call a `uris`-skill launch uses.
fn sync_ohos_shortcuts(project: &Project) -> Result<(), String> {
    let module = staged_harmony_dir(project).join("entry/src/main/module.json5");
    if !module.exists() {
        return Ok(());
    }
    let resources = staged_harmony_dir(project).join("entry/src/main/resources");
    let profile = resources.join("base/profile/shortcuts_config.json");
    let shortcuts = crate::shortcuts::resolved(project)?;
    clear_day_strings(&resources, "day_shortcut_")?;
    if shortcuts.is_empty() {
        // Keep an existing metadata reference valid, drop the owned strings everywhere.
        if profile.exists() {
            let empty = crate::shortcuts::harmony_shortcuts_config(&[], None, "", "", "");
            std::fs::write(&profile, empty).map_err(|e| format!("{}: {e}", profile.display()))?;
        }
        return Ok(());
    }

    let text =
        std::fs::read_to_string(&module).map_err(|e| format!("{}: {e}", module.display()))?;
    let (module_name, scheme, after) =
        shortcut_module(&text).map_err(|e| format!("{}: {e}", module.display()))?;
    let bundle = project.manifest.resolve("harmony-arkui").id;

    let config = crate::shortcuts::harmony_shortcuts_config(
        &shortcuts,
        scheme.as_deref(),
        &bundle,
        &module_name,
        "EntryAbility",
    );
    if let Some(parent) = profile.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let stale = std::fs::read_to_string(&profile).ok();
    if stale.as_deref() != Some(config.as_str()) {
        std::fs::write(&profile, config).map_err(|e| format!("{}: {e}", profile.display()))?;
    }

    if after != text {
        std::fs::write(&module, after).map_err(|e| format!("{}: {e}", module.display()))?;
    }

    // Labels per locale: `base` carries the default locale, others get their qualifier dir
    // (created on first use; `merge_day_strings` preserves anything an app put there itself).
    for loc in shortcuts[0].labels.keys() {
        let mut entries = std::collections::BTreeMap::new();
        for sc in &shortcuts {
            let label = sc.labels.get(loc).unwrap_or(&sc.base);
            entries.insert(sc.id.clone(), label.clone());
        }
        let dir = crate::shortcuts::harmony_resource_dir(loc);
        merge_day_strings(
            &resources.join(dir).join("element/string.json"),
            "day_shortcut_",
            &entries,
        )?;
    }
    Ok(())
}

/// Prepare the writable hvigor project for builds and DevEco Studio. The checked-in host
/// provides the inputs; all Day-generated ArkTS, resources and manifest edits go to the copy.
/// Native caches survive preparation. Removed source files and generated resources do not.
pub fn stage_host(project: &Project) -> Result<(), String> {
    let source = harmony_dir(project);
    if !source.join("build-profile.json5").exists() {
        return Ok(());
    }
    let harmony = staged_harmony_dir(project);
    staging::sync(&source, &harmony)?;
    crate::pieces::write_ohos_pieces(project, &harmony)?;
    let target = crate::external::find_target(project, "harmony-arkui")?;
    crate::resources::stage(project, target)?;
    sync_ohos_identity(project)?;
    sync_ohos_permissions(project)?;
    sync_ohos_shortcuts(project)
}

pub fn build_ohos(
    project: &Project,
    target: &'static Target,
    profile: Profile,
    start: std::time::Instant,
) -> Result<BuildOutcome, String> {
    let harmony = harmony_dir(project);
    if !harmony.join("build-profile.json5").exists() {
        return Err(format!(
            "harmony-arkui: no ArkTS host project at {} — a HarmonyOS app needs a `platform/harmony/` \
             hvigor project, the one `day new` scaffolds (`day project add-target harmony-arkui`). See \
             docs/harmonyos.md.",
            harmony.display()
        ));
    }

    // 0) Stage the framework's ArkTS host (docs/harmonyos.md) and every standalone piece's ArkTS
    //    into the project, and regenerate the aggregator the host page registers
    //    (docs/extending.md). Before the cargo leg, because hvigor compiles whatever is on disk
    //    and a piece's Rust renderer is useless without its ArkTS half.
    stage_host(project)?;
    let harmony = staged_harmony_dir(project);

    // 1) Cross-compile the app to a cdylib for each connected target's arch (an emulator is
    //    x86_64, a device arm64; the hap carries both so it installs on either), staging each as
    //    entry/libs/<abi>/libentry.so, the .so the ArkTS host imports (its NAPI module is
    //    "entry"). Uses the OHOS NDK cross-linker (OHOS_NDK_HOME) + a rustup toolchain (Homebrew
    //    rustc ships no OHOS std) and `feature_selection("arkui")` (the arkui toolkit feature +
    //    every standalone piece's `<pkg>/arkui` renderer feature, Tier A.2), exactly like the
    //    android/iOS legs.
    let ndk = find_ohos_ndk()?;
    let (cargo, bin) = rustup_cargo()?;
    let name = project.manifest.app.name.clone();
    // Drop any previously staged arch before restaging. hvigor packs whatever `entry/libs` holds,
    // and these directories are never otherwise cleaned, so an earlier x86_64 emulator build left
    // its .so behind and rode into the hap alongside (or instead of) the arch just built. The hap
    // must contain exactly what this invocation produced (§20.3).
    let libs_root = harmony.join("entry/libs");
    if libs_root.is_dir() {
        std::fs::remove_dir_all(&libs_root)
            .map_err(|e| format!("clearing {}: {e}", libs_root.display()))?;
    }
    let arches = ohos_build_arches();
    let triples: Vec<&str> = arches.iter().map(|(t, _)| *t).collect();
    crate::ops::ensure_rust_targets(&triples)?;
    for (triple, abi) in arches {
        let target_dir = crate::ops::build_root(project)
            .join("cargo/harmony-arkui")
            .join(abi)
            .join(profile.as_str());
        let linker_var = format!(
            "CARGO_TARGET_{}_LINKER",
            triple.to_uppercase().replace('-', "_")
        );
        status("Building", &format!("{} (cargo cdylib {abi})", target.name));
        let mut cmd = Command::new(&cargo);
        crate::patch::apply_day_src(&mut cmd);
        crate::ops::apply_app_identity(&mut cmd, project, target.name);
        // The hvigor module staged before this build carries every bridged crate's ArkTS arm, so
        // the cfg that switches those arms on rides the same cargo run (docs/bridge.md).
        crate::bridge::apply_staged(&mut cmd, project, "harmony-arkui");
        cmd.current_dir(&project.root)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("CARGO_TARGET_DIR", &target_dir)
            .env(&linker_var, format!("{ndk}/llvm/bin/{triple}-clang"))
            // day-arkui-sys's build.rs compiles the C++ shim with the NDK clang and reads this
            // variable itself; export the resolved path so auto-detected local installs work even
            // when the parent environment (a GUI-launched editor) never set it.
            .env("OHOS_NDK_HOME", &ndk)
            // cc-rs (used by build scripts of C-carrying deps, e.g. ring under day-part-http's
            // fallback TLS) picks the cross compiler from these per-target vars; without them it
            // falls back to the host `cc`, which can't target ohos.
            .env(
                format!("CC_{}", triple.replace('-', "_")),
                format!("{ndk}/llvm/bin/{triple}-clang"),
            )
            .env(
                format!("AR_{}", triple.replace('-', "_")),
                format!("{ndk}/llvm/bin/llvm-ar"),
            )
            // bindgen (rquickjs-sys under daybrite/day-lite, its docs/lite.md §13) runs the host
            // libclang, which inherits neither the CC_* wrapper nor its sysroot, so feed it the
            // same flags the NDK's `<triple>-clang` wrapper script passes (`-unknown` dropped from
            // the clang -target, per the wrapper).
            .env(
                format!("BINDGEN_EXTRA_CLANG_ARGS_{}", triple.replace('-', "_")),
                format!(
                    "--target={} --sysroot={ndk}/sysroot -D__MUSL__",
                    triple.replace("-unknown", "")
                ),
            )
            .args([
                "rustc",
                "-p",
                &name,
                "--lib",
                "--crate-type",
                "cdylib",
                "--no-default-features",
                "--features",
                &crate::ops::feature_selection(project, "arkui"),
                "--target",
                triple,
            ]);
        if profile == Profile::Release {
            cmd.arg("--release");
        }
        run_logged(&mut cmd, &format!("cargo (ohos {abi})"))?;
        // The cdylib is `lib<[lib].name>.so` (libentry.so for a crate whose `[lib] name = "entry"`,
        // else lib<crate>.so), so find the single produced .so and stage it as libentry.so.
        let out_dir = target_dir.join(triple).join(profile.as_str());
        let so = std::fs::read_dir(&out_dir)
            .map_err(|e| format!("reading {}: {e}", out_dir.display()))?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("so"))
            .ok_or_else(|| format!("no cdylib .so produced in {}", out_dir.display()))?;
        let libs = harmony.join("entry/libs").join(abi);
        std::fs::create_dir_all(&libs).map_err(|e| format!("mkdir {}: {e}", libs.display()))?;
        std::fs::copy(&so, libs.join("libentry.so"))
            .map_err(|e| format!("stage libentry.so: {e}"))?;
        // libentry.so links the NDK's shared libc++ (the day-arkui-sys C++ shim), which
        // OpenHarmony does not provide on-device for apps: an unbundled hap dies at load with
        // MUSL-LDSO's "Error loading shared library libc++_shared.so". Stage it next to
        // libentry.so so hvigor packs it into the hap (the exact analogue of the Android jniLibs
        // bundling). The NDK's per-arch lib dir uses the clang triple (`x86_64-linux-ohos`), not
        // the Rust triple, so drop the `unknown-` vendor field.
        let clang_triple = triple.replace("unknown-", "");
        let libcxx = PathBuf::from(&ndk)
            .join("llvm/lib")
            .join(&clang_triple)
            .join("libc++_shared.so");
        if libcxx.exists() {
            std::fs::copy(&libcxx, libs.join("libc++_shared.so"))
                .map_err(|e| format!("stage libc++_shared.so: {e}"))?;
        } else {
            status(
                "Warning",
                &format!(
                    "libc++_shared.so not found at {} — the hap may fail to load",
                    libcxx.display()
                ),
            );
        }
    }

    // 2) Assemble the .hap with hvigor (compiles the ArkTS host + packs the native libs + resources).
    //    hvigor + ohpm come from the OpenHarmony command-line-tools (on PATH); the SDK from
    //    OHOS_BASE_SDK_HOME. `ohpm install` is best-effort (the app has only a local dependency).
    status(
        "Building",
        &format!("{} (hvigorw assembleHap)", target.name),
    );
    let _ = Command::new(find_tool("ohpm").unwrap_or_else(|| "ohpm".into()))
        .arg("install")
        .current_dir(&harmony)
        .status();

    let mode = profile.as_str();
    // A missing hvigor otherwise surfaces as a bare spawn ENOENT, so check up front and say what to
    // install (it is not part of the public SDK; the `native` NDK alone only covers the Rust step).
    let Some(hvigorw) = find_tool("hvigorw") else {
        return Err(
            "hvigorw not found on PATH — the Rust cross-compile succeeded, but packaging the .hap \
             needs the OpenHarmony command-line-tools (hvigor + ohpm; bundled with DevEco Studio). \
             Install them and put their bin/ on PATH — see docs/harmonyos.md."
                .into(),
        );
    };
    let mut hv = Command::new(hvigorw);
    hv.current_dir(&harmony).args([
        "assembleHap",
        "--mode",
        "module",
        "-p",
        "product=default",
        "-p",
        &format!("buildMode={mode}"),
        "--no-daemon",
    ]);
    // Bounded like the gradle leg: hvigor on a wedged emulator query must not outlive the
    // build ceiling.
    crate::mobile::run_logged_within(&mut hv, "hvigorw assembleHap", crate::ops::BUILD_TIMEOUT)?;

    // 3) Patch + sign the assembled (unsigned) .hap via sign-hap.mjs: it rewrites module.json's
    //    compileSdkType to "OpenHarmony" (so the emulator skips code-sign verification; see the
    //    script) then signs with the OpenHarmony public release material.
    let hap = sign_hap(project, &harmony, &ndk)?;
    status("Built", &format!("{} → {}", target.name, hap.display()));
    Ok(BuildOutcome {
        target: target.name,
        artifact: hap,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// The hvigor-built unsigned hap of `project` (the release re-signing input; pack/ohos.rs).
pub(crate) fn find_unsigned_hap(project: &crate::meta::Project) -> Option<PathBuf> {
    find_hap(&staged_harmony_dir(project).join("entry/build"), |n| {
        n.contains("unsigned")
    })
}

/// Recursively find the first `*.hap` under `dir` whose file name satisfies `pred`.
fn find_hap(dir: &Path, pred: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("hap") {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if pred(name) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// The dev-tier patch + sign script (`node sign-hap.mjs <unsigned> <signed>`, cwd = the hvigor
/// project, which it reads AppScope/app.json5 from). The CLI's own: it is tooling, not app
/// code, so it ships embedded here and is written under `build/day/harmony/` when a build needs
/// it; a project that still carries a `platform/harmony/sign-hap.mjs` of its own (the
/// pre-2026-09 scaffold) keeps using that one.
const SIGN_HAP_MJS: &str = include_str!("../resources/harmony/sign-hap.mjs");

/// Patch + sign the hvigor-built unsigned hap via `sign-hap.mjs <unsigned> <signed>` (Node;
/// hvigor already requires it). The script rewrites module.json's compileSdkType to
/// "OpenHarmony" so the emulator skips code-sign verification (the public release cert's code
/// signature is otherwise rejected with 9568393), then signs with the SDK's release material.
fn sign_hap(project: &Project, harmony: &Path, ndk: &str) -> Result<PathBuf, String> {
    let build = harmony.join("entry/build");
    // hvigor emits `entry-<product>-unsigned.hap`; fall back to any hap.
    let unsigned = find_hap(&build, |n| n.contains("unsigned"))
        .or_else(|| find_hap(&build, |_| true))
        .ok_or_else(|| format!("no .hap produced under {}", build.display()))?;
    let own = harmony.join("sign-hap.mjs");
    let sign = if own.exists() {
        own
    } else {
        let staged = project.root.join("build/day/harmony/sign-hap.mjs");
        if let Some(dir) = staged.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        crate::pieces::write_if_changed(&staged, SIGN_HAP_MJS)?;
        staged
    };
    let signed = unsigned.with_file_name("day-signed.hap");
    status("Signing", &signed.display().to_string());
    let mut cmd = Command::new("node");
    cmd.arg(&sign)
        .arg(&unsigned)
        .arg(&signed)
        // The script locates the SDK signing material relative to the NDK (its findLib probes
        // OHOS_NDK_HOME first), so hand it the resolved path, like the cargo step.
        .env("OHOS_NDK_HOME", ndk)
        .current_dir(harmony);
    run_logged(&mut cmd, "sign-hap.mjs")?;
    Ok(signed)
}

/// Combined stdout+stderr of a finished command, as one string.
fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Is `bundle` installed on the target? `hdc install`/`bm install` can print `error: failed to
/// execute your command` yet still install (and yet exit 0), so verify the end state with
/// `bm dump -a`, the flat list of every installed bundle name: a clean membership test (unlike
/// `bm dump -n <bundle>`, whose per-bundle JSON can itself contain the words "error"/"failed").
fn bundle_installed_on(bundle: &str, key: &str) -> bool {
    // Bounded: `bm dump` against a wedged guest waits like every other hdc call, and this runs
    // inside the install retry loop; an unanswered probe reads as "not installed yet".
    crate::ops::output_within(
        hdc_for(key).args(["shell", "bm", "dump", "-a"]),
        LAUNCH_TIMEOUT,
    )
    .map(|o| combined(&o).contains(bundle))
    .unwrap_or(false)
}

pub fn launch_ohos(
    project: &Project,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<std::thread::JoinHandle<i32>, String> {
    let bundle = project.manifest.app.id.clone();
    // Recorded before enumerating: `ohos_devices` narrows to it, and so do the dayscript forward
    // and capture steps that run later with no spec in hand.
    if let Some(key) = spec.ohos_device.as_deref() {
        crate::ops::remember_ohos_key(key);
    }
    let devices = ohos_devices();
    if devices.is_empty() {
        return Err(match spec.ohos_device.as_deref() {
            Some(key) => format!(
                "OpenHarmony device {key:?} is not reachable (check `day devices list`, or \
                 `hdc list targets`)"
            ),
            None => format!(
                "no OpenHarmony target reachable (hdc). Boot an emulator \
                 (`day devices boot -p harmony-arkui`) or attach a device; the default connect key is {}.",
                ohos_target()
            ),
        });
    }
    if let [only] = devices.as_slice() {
        crate::ops::remember_ohos_key(only.key.clone());
    }
    // The dayscript runner drives one target over the hdc-forwarded port (the default key), so a
    // scripted run stays deterministic even with several targets attached.
    let multi = devices.len() > 1;
    let mut log_threads = Vec::new();
    for dev in &devices {
        install_and_start(&bundle, &dev.key, outcome, spec)?;
        if spec.attached {
            let label = if multi {
                format!("{}:{}", outcome.target, dev.key)
            } else {
                outcome.target.to_string()
            };
            let key = dev.key.clone();
            log_threads.push(std::thread::spawn(move || stream_hilog(&key, &label)));
        }
    }
    Ok(std::thread::spawn(move || {
        let mut code = 0;
        for t in log_threads {
            if let Ok(c) = t.join()
                && c != 0
                && code == 0
            {
                code = c;
            }
        }
        code
    }))
}

/// Install (reinstall) + `aa start` the bundle on the target `key`, with the Oniro retry dances.
fn install_and_start(
    bundle: &str,
    key: &str,
    outcome: &BuildOutcome,
    spec: &LaunchSpec,
) -> Result<(), String> {
    // Keep the screen awake + in never-doze power mode so it doesn't re-lock mid-run (best-effort).
    // The timeout override is i32::MAX (~24 days), not a session-sized number: once the display
    // sleeps the keyguard returns, `uitest screenCap` captures black frames, and, worse, a
    // long-idle guest refuses the unlock swipe outright ("developer mode … cannot be unlocked
    // automatically"), stranding every later `aa start` until the emulator is rebooted.
    let _ = hdc_for(key)
        .args(["shell", "power-shell", "wakeup"])
        .status();
    let _ = hdc_for(key)
        .args(["shell", "power-shell", "setmode", "602"])
        .status();
    let _ = hdc_for(key)
        .args(["shell", "power-shell", "timeout", "-o", "2147483647"])
        .status();
    unlock_keyguard(key);

    // Install (reinstall over any existing copy), RETRYING: right after boot the bundle-manager
    // service may not accept installs yet, and `hdc install`'s exit code + its "error: failed to
    // execute your command" message are both unreliable on Oniro (the app often installs anyway).
    // Gate on `bm dump -a` actually listing the bundle rather than on the install command's output.
    status("Installing", &format!("harmony-arkui ({bundle}) on {key}"));
    let mut install_log = String::new();
    let mut installed = false;
    for attempt in 1..=10u32 {
        // Bounded (ops.rs INSTALL_TIMEOUT): hdc waits for a wedged guest with no deadline of
        // its own, and the `bm dump` gate below decides success anyway.
        if let Some(out) = crate::ops::output_within(
            hdc_for(key).args(["install", "-r"]).arg(&outcome.artifact),
            INSTALL_TIMEOUT,
        ) {
            install_log = combined(&out);
        }
        if bundle_installed_on(bundle, key) {
            installed = true;
            break;
        }
        if attempt < 10 {
            let _ = hdc_for(key)
                .args(["shell", "power-shell", "wakeup"])
                .status();
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    if !installed {
        return Err(format!(
            "hdc install: {bundle} not installed on {key} after 10 tries:\n{}",
            install_log.trim()
        ));
    }

    // The `aa start` args: the dayscript engine port/token + locale as `--ps` string parameters
    // (all shell-safe single tokens). EntryAbility.ets applies them to the process env (via the
    // native `setEnv`) before `start()` runs the engine; this mirrors Android's intent extras.
    let mut args: Vec<String> = ["shell", "aa", "start", "-a", "EntryAbility", "-b", bundle]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (k, v) in &spec.envs {
        let param = match k.as_str() {
            "DAYSCRIPT_PORT" => "day.dayscript.port".to_string(),
            "DAYSCRIPT_TOKEN" => "day.dayscript.token".to_string(),
            other => format!("day.env.{other}"),
        };
        args.extend(["--ps".to_string(), param, v.clone()]);
    }
    if let Some(locale) = &spec.locale {
        args.extend(["--ps".to_string(), "day.locale".to_string(), locale.clone()]);
    }

    status("Launching", &format!("harmony-arkui ({bundle}) on {key}"));
    // Kill any running instance first: the ability is a singleton, so a bare `aa start` would
    // just foreground it, with the old run's dayscript port/token, while this run's engine
    // params ride the new want. A fresh process re-reads them in onCreate (docs/harmonyos.md).
    let _ = hdc_for(key)
        .args(["shell", "aa", "force-stop", bundle])
        .status();
    std::thread::sleep(Duration::from_secs(2));
    // The emulator boots with the keyguard up, and the keyguard returns whenever the display
    // sleeps; `aa start` is refused while it shows (Error 10106102: "developer mode … cannot be
    // unlocked automatically"; there is no hdc force-unlock). But the lock screen is
    // slide-to-unlock, so a synthetic swipe dismisses it (see `unlock_keyguard`). Retry,
    // re-waking + re-swiping between tries. `aa start` also exits 0 even when refused, so we
    // inspect its output for the failure markers.
    // 40 tries × 3s ≈ 2 min of retries: a fresh userdata's first boot renders the keyguard
    // late on a slow TCG guest (CI), and `aa start` is refused until the swipe can land.
    let mut last = String::new();
    for attempt in 1..=40u32 {
        // Bounded per try (ops.rs LAUNCH_TIMEOUT): the retry loop already owns the patience.
        let out = crate::ops::output_within(hdc_for(key).args(&args), LAUNCH_TIMEOUT)
            .ok_or_else(|| crate::ops::timeout_message("hdc aa start", LAUNCH_TIMEOUT))?;
        let text = combined(&out);
        if out.status.success()
            && !text.contains("Error Code:")
            && !text.to_lowercase().contains("failed to start")
        {
            return Ok(());
        }
        last = text;
        if attempt < 40 {
            let _ = hdc_for(key)
                .args(["shell", "power-shell", "wakeup"])
                .status();
            unlock_keyguard(key);
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    Err(format!(
        "hdc aa start refused on {key} after 40 tries (keyguard/launch):\n{}",
        last.trim()
    ))
}

/// Dismiss the slide-to-unlock keyguard with a synthetic swipe-up (best-effort): up the middle
/// of the panel, from five sixths of its height to one seventh. On the 360×720 phone panel that
/// is (180, 600) to (180, 102), the swipe verified headlessly: after `power-shell wakeup` the
/// lock screen shows "Please slide to unlock", and it lands on the home screen. On an unlocked
/// screen the swipe is a harmless scroll. The panel is the guest's own screen size, asked of its
/// RenderService: the size `emulator_launch` requested is not always what the guest runs at,
/// since QEMU's GTK window (a windowed boot on Linux) hands virtio-gpu its own 640×480, and a
/// swipe aimed at the requested 360×720 then starts below the screen and never unlocks. When the
/// query fails the requested panel is used, read back from `DAY_OHOS_PANEL`; unset means the
/// phone. Both injection drivers are tried, `uitest uiInput` (test daemon; slow to spin up on a
/// cold TCG guest) and `uinput` (kernel-level, no daemon), because a slow first boot can leave
/// the daemon unready while the keyguard is already up.
fn unlock_keyguard(key: &str) {
    let (w, h) = guest_screen_size(key)
        .or_else(|| {
            let v = std::env::var("DAY_OHOS_PANEL").ok()?;
            parse_size(&v)
        })
        .unwrap_or(crate::devices::HARMONY_PHONE_PANEL);
    let x = (w / 2).to_string();
    let from = (h * 5 / 6).to_string();
    let to = (h / 7).to_string();
    let _ = hdc_for(key)
        .args([
            "shell", "uitest", "uiInput", "swipe", &x, &from, &x, &to, "500",
        ])
        .status();
    let _ = hdc_for(key)
        .args(["shell", "uinput", "-T", "-m", &x, &from, &x, &to, "300"])
        .status();
}

/// The guest's current screen size, from RenderService's screen dump (best-effort).
fn guest_screen_size(key: &str) -> Option<(u32, u32)> {
    let out = hdc_for(key)
        .args(["shell", "hidumper", "-s", "RenderService", "-a", "screen"])
        .output()
        .ok()?;
    parse_screen_dump(&String::from_utf8_lossy(&out.stdout))
}

/// The first screen's `physical resolution=WxH` in a RenderService screen dump.
fn parse_screen_dump(dump: &str) -> Option<(u32, u32)> {
    let rest = dump.split("physical resolution=").nth(1)?;
    let size = rest.split(|c: char| c == ',' || c.is_whitespace()).next()?;
    parse_size(size)
}

/// A `WxH` size with both sides non-zero.
fn parse_size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.split_once('x')?;
    let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// Stream one target's hilog into the day log with `label` (best-effort). Returns its exit code.
fn stream_hilog(key: &str, label: &str) -> i32 {
    match hdc_for(key)
        .args(["shell", "hilog"])
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            crate::signals::register_child(child.id());
            if let Some(out) = child.stdout.take() {
                for line in
                    std::io::BufRead::lines(std::io::BufReader::new(out)).map_while(Result::ok)
                {
                    emit_log(label, LogStream::Out, &line);
                }
            }
            child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0)
        }
        Err(e) => {
            emit_log(label, LogStream::Err, &format!("hdc hilog: {e}"));
            1
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::replace_json5_string;

    #[test]
    fn identity_ignores_comments_and_handles_json5_escapes() {
        let src = r#"// "scheme": "leave-me"
{scheme: 'a\'b', note: 'scheme: "untouched"', nested: {'scheme': 'old'}}"#;
        let out = replace_json5_string(src, "scheme", "new\"value").unwrap();
        assert!(out.starts_with("// \"scheme\": \"leave-me\"\n"));
        assert!(out.contains("note: 'scheme: \"untouched\"'"));
        let parsed: serde_json::Value = json_five::from_str(&out).unwrap();
        assert_eq!(parsed["scheme"], "new\"value");
        assert_eq!(parsed["nested"]["scheme"], "new\"value");
        assert_eq!(
            replace_json5_string(&out, "scheme", "new\"value").unwrap(),
            out
        );
        assert!(replace_json5_string("{scheme:", "scheme", "x").is_err());
    }

    #[test]
    fn shortcuts_use_the_owning_ability_and_merge_metadata() {
        let src = r#"// "name": "EntryAbility", "scheme": "wrong"
{module: {name:'custom', note:'ohos.ability.shortcuts', abilities:[
  {name:'OtherAbility', skills:[{uris:[{scheme:'wrong'}]}]},
  {name:'EntryAbility', metadata:[{name:'other', resource:'keep'}],
   skills:[{uris:[{scheme:'right'}]}]}
]}}"#;
        let (name, scheme, out) = super::shortcut_module(src).unwrap();
        assert_eq!(name, "custom");
        assert_eq!(scheme.as_deref(), Some("right"));
        assert!(out.starts_with("// \"name\": \"EntryAbility\", \"scheme\": \"wrong\"\n"));
        assert!(out.contains("{name:'other', resource:'keep'}"));
        let parsed: serde_json::Value = json_five::from_str(&out).unwrap();
        assert!(parsed["module"]["abilities"][0].get("metadata").is_none());
        assert_eq!(
            parsed["module"]["abilities"][1]["metadata"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(super::shortcut_module(&out).unwrap().2, out);
    }

    #[test]
    fn shortcuts_add_missing_metadata_and_repair_existing_reference() {
        for metadata in [
            "",
            ", metadata: [{name:'ohos.ability.shortcuts', resource:'old'}]",
        ] {
            let src = format!(
                "{{module:{{name:'entry',abilities:[{{name:'EntryAbility'{metadata}}}]}}}}"
            );
            let (_, scheme, out) = super::shortcut_module(&src).unwrap();
            assert_eq!(scheme, None);
            let parsed: serde_json::Value = json_five::from_str(&out).unwrap();
            assert_eq!(
                parsed["module"]["abilities"][0]["metadata"][0]["resource"],
                "$profile:shortcuts_config"
            );
            assert_eq!(super::shortcut_module(&out).unwrap().2, out);
        }
        assert!(
            super::shortcut_module("{module:{name:'entry',abilities:[]}} // name: 'EntryAbility'")
                .is_err()
        );
    }

    /// The rewrite touches the one field and leaves the comments, trailing commas and spacing
    /// a hand-edited JSON5 file carries, using the parser’s round-trip representation.
    #[test]
    fn only_the_named_field_moves() {
        let src = "{\n  \"app\": {\n    // the app's id\n    \"bundleName\": \"dev.example.old\",\n    \"vendor\": \"example\",\n  }\n}\n";
        let out = replace_json5_string(src, "bundleName", "dev.daybrite.new").unwrap();
        assert!(out.contains("\"bundleName\": \"dev.daybrite.new\""));
        assert!(out.contains("// the app's id"));
        assert!(out.contains("\"vendor\": \"example\","));
        assert_eq!(out.lines().count(), src.lines().count());
    }

    /// Every occurrence, because the OHOS ability declares its skill `uris` as a list.
    #[test]
    fn every_occurrence_is_replaced() {
        let src = "{ \"uris\": [{ \"scheme\": \"a\" }, { \"scheme\": \"a\" }] }";
        assert_eq!(
            replace_json5_string(src, "scheme", "b").unwrap(),
            "{ \"uris\": [{ \"scheme\": \"b\" }, { \"scheme\": \"b\" }] }"
        );
    }

    /// A non-string value (or a key that is only mentioned) is left exactly as it was.
    #[test]
    fn non_string_values_are_untouched() {
        let src = "{ \"scheme\": 7, \"note\": \"scheme is derived\" }";
        assert_eq!(replace_json5_string(src, "scheme", "b").unwrap(), src);
    }
}

#[cfg(test)]
mod screen_size_tests {
    use super::{parse_screen_dump, parse_size};

    #[test]
    fn reads_the_physical_resolution_from_a_screen_dump() {
        // Captured from an Oniro v6.1 guest booted in a QEMU GTK window.
        let dump = "screen[0]: id=0, powerStatus=POWER_STATUS_ON, backlight=-1, \
                    screenType=EXTERNAL_TYPE, render resolution=640x480, \
                    physical resolution=640x480, isVirtual=false\nactiveMode: 640x480, refreshRate=60";
        assert_eq!(parse_screen_dump(dump), Some((640, 480)));
    }

    #[test]
    fn a_dump_without_a_screen_is_none() {
        assert_eq!(parse_screen_dump(""), None);
        assert_eq!(parse_screen_dump("error: no such ability"), None);
    }

    #[test]
    fn sizes_must_be_two_positive_numbers() {
        assert_eq!(parse_size("360x720"), Some((360, 720)));
        assert_eq!(parse_size("0x720"), None);
        assert_eq!(parse_size("360"), None);
        assert_eq!(parse_size("wide"), None);
    }
}

#[cfg(test)]
mod emulator_image_tests {
    use super::EmulatorImage;
    use std::path::PathBuf;

    fn scratch(tag: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "day-emu-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in files {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn layouts_are_told_apart_by_their_files() {
        let oniro = scratch("oniro", EmulatorImage::Oniro.files());
        assert_eq!(EmulatorImage::detect(&oniro), Ok(EmulatorImage::Oniro));
        assert!(EmulatorImage::Oniro.missing(&oniro).is_empty());

        let seven = scratch("ohos-qemu", EmulatorImage::OhosQemu.files());
        assert_eq!(EmulatorImage::detect(&seven), Ok(EmulatorImage::OhosQemu));
        assert!(EmulatorImage::OhosQemu.missing(&seven).is_empty());

        // One extra partition is enough to call it ohos-qemu, and then the rest are reported.
        let partial = scratch("partial", &["bzImage", "sys_prod.img"]);
        assert_eq!(EmulatorImage::detect(&partial), Ok(EmulatorImage::OhosQemu));
        assert!(
            EmulatorImage::OhosQemu
                .missing(&partial)
                .contains(&"chip_prod.img")
        );

        // ohos-qemu's arm64 package is refused by name rather than half-booted.
        let arm = scratch("arm64", &["Image", "ramdisk.img", "system.img"]);
        assert!(EmulatorImage::detect(&arm).unwrap_err().contains("arm64"));

        // An empty directory reads as Oniro with everything missing.
        let empty = scratch("empty", &[]);
        assert_eq!(EmulatorImage::detect(&empty), Ok(EmulatorImage::Oniro));
        assert_eq!(
            EmulatorImage::Oniro.missing(&empty).len(),
            EmulatorImage::Oniro.files().len()
        );
        for d in [oniro, seven, partial, arm, empty] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// Each `ohos.required_mount.<mount>=/dev/block/vdX` in the command line must name the disk
    /// at position X of the QEMU command, and every disk file must be one the layout ships.
    /// A mismatch doesn't fail loudly: the guest mounts the wrong partition and never boots.
    #[test]
    fn kernel_mounts_match_the_disk_order() {
        for image in [EmulatorImage::Oniro, EmulatorImage::OhosQemu] {
            let disks = image.disks();
            for disk in disks {
                assert!(
                    image.files().contains(&format!("{disk}.img").as_str()),
                    "{image:?}: {disk}.img is not in files()"
                );
            }
            let mounts: Vec<(&str, usize)> = image
                .append()
                .split_whitespace()
                .filter_map(|arg| {
                    let rest = arg.strip_prefix("ohos.required_mount.")?;
                    let (mount, spec) = rest.split_once('=')?;
                    let dev = spec.strip_prefix("/dev/block/vd")?.chars().next()?;
                    Some((mount, (dev as u8 - b'a') as usize))
                })
                .collect();
            assert!(!mounts.is_empty(), "{image:?}: no mounts parsed");
            for (mount, index) in mounts {
                let disk = disks[index];
                let expected = match mount {
                    "misc" => "updater",
                    "data" => "userdata",
                    other => other,
                };
                assert_eq!(disk, expected, "{image:?}: {mount} is vd{index}");
            }
            // The drives in the QEMU arguments come in the same order.
            let args = image.qemu_args();
            let drives: Vec<&str> = args
                .iter()
                .filter_map(|a| a.strip_prefix("if=none,file="))
                .filter_map(|a| a.split_once(".img").map(|(f, _)| f))
                .collect();
            assert_eq!(drives, disks, "{image:?}");
        }
    }

    #[test]
    fn guest_hdc_ports_differ_by_layout() {
        assert_eq!(EmulatorImage::Oniro.guest_hdc_port(), 55555);
        assert_eq!(EmulatorImage::OhosQemu.guest_hdc_port(), 5555);
    }
}
