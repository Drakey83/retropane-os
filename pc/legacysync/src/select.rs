//! Choosing the CAB for the device, as Application Manager does: by the
//! processor and OS versions in each CAB's setup script, not by its name
//! (docs/legacysync/activesync-interfaces.md §3).

/// What the device reports about itself.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    /// The OS version programs see (major, minor).
    pub os: (u32, u32),
    /// The platform name scripts may list as unsupported.
    pub platform: &'static str,
}

/// RetroPane on the X51v: it runs Windows Mobile 5 programs, so it reports
/// what Windows Mobile 5 did, CE 5.1 on a Pocket PC.
pub const X51V: Device = Device {
    os: (5, 1),
    platform: "PocketPC",
};

/// Processor types (public `winnt.h` values).
pub mod cpu {
    /// The script runs on any processor.
    pub const ANY: u32 = 0;
    /// StrongARM: what ARM Pocket PCs (and the X51v's XScale) report.
    pub const STRONGARM: u32 = 2577;
    /// Older ARM cores; the X51v's XScale (ARMv5TE) runs their code.
    pub const OLDER_ARM: [u32; 4] = [1824, 2080, 2336, 70001];
}

/// What a script says about where it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirements {
    /// Processor type.
    pub processor: u32,
    /// Oldest OS (major, minor).
    pub os_min: (u32, u32),
    /// Newest OS (major, minor).
    pub os_max: (u32, u32),
    /// Platforms it refuses.
    pub unsupported: Vec<String>,
}

/// How well a CAB fits the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CpuFit {
    /// Built for another processor (MIPS, SH, x86): can't run.
    Other,
    /// Processor-independent.
    Any,
    /// An older ARM core: runs.
    OlderArm,
    /// ARM/StrongARM: the usual Pocket PC build.
    Arm,
}

/// The verdict on one CAB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fit {
    /// Processor match.
    pub cpu: CpuFit,
    /// The device's OS version is in the script's range. A CAB outside it
    /// may still work (GeoRally says 4.0-4.99 and is a WM5-era game), so
    /// it can be chosen, with a warning, when nothing better exists.
    pub version_ok: bool,
    /// The script doesn't list the device's platform as unsupported.
    pub platform_ok: bool,
}

impl Fit {
    /// Whether it can be installed at all.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.cpu != CpuFit::Other && self.platform_ok
    }
}

/// Judges one script against the device.
#[must_use]
pub fn assess(req: &Requirements, device: &Device) -> Fit {
    let cpu = match req.processor {
        cpu::STRONGARM => CpuFit::Arm,
        cpu::ANY => CpuFit::Any,
        p if cpu::OLDER_ARM.contains(&p) => CpuFit::OlderArm,
        _ => CpuFit::Other,
    };
    // A max of 0.0 (Dell's Diagnostic CAB) means "no limit".
    let below_max = req.os_max == (0, 0) || device.os <= req.os_max;
    let version_ok = req.os_min <= device.os && below_max;
    let platform_ok = !req
        .unsupported
        .iter()
        .any(|p| p.eq_ignore_ascii_case(device.platform));
    Fit {
        cpu,
        version_ok,
        platform_ok,
    }
}

/// The best usable CAB among `fits` (in INI order): in-range versions
/// first, then the closest processor match; ties go to the INI's order.
#[must_use]
pub fn choose(fits: &[Option<Fit>]) -> Option<usize> {
    fits.iter()
        .enumerate()
        .filter_map(|(i, f)| f.filter(Fit::usable).map(|f| (i, f)))
        .max_by_key(|&(i, f)| (f.version_ok, f.cpu, core::cmp::Reverse(i)))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(
        processor: u32,
        os_min: (u32, u32),
        os_max: (u32, u32),
        unsupported: &[&str],
    ) -> Requirements {
        Requirements {
            processor,
            os_min,
            os_max,
            unsupported: unsupported.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn the_real_cabs_are_judged_as_expected() {
        // Toy Golf: any CPU, 4.20-5.99.
        let f = assess(&req(0, (4, 20), (5, 99), &[]), &X51V);
        assert_eq!(
            f,
            Fit {
                cpu: CpuFit::Any,
                version_ok: true,
                platform_ok: true
            }
        );
        // Virtual Pool Mobile: StrongARM, 3.0-999.0, refuses HPC/JUPITER.
        let f = assess(&req(2577, (3, 0), (999, 0), &["JUPITER", "HPC"]), &X51V);
        assert_eq!(
            f,
            Fit {
                cpu: CpuFit::Arm,
                version_ok: true,
                platform_ok: true
            }
        );
        // GeoRally: 4.0-4.99, so CE 5.1 is outside its range; still usable.
        let f = assess(&req(0, (4, 0), (4, 99), &[]), &X51V);
        assert!(!f.version_ok && f.usable());
        // Diagnostic: no version limits at all.
        assert!(assess(&req(0, (0, 0), (0, 0), &[]), &X51V).version_ok);
    }

    #[test]
    fn other_processors_and_refused_platforms_are_unusable() {
        assert!(!assess(&req(4000, (3, 0), (5, 99), &[]), &X51V).usable()); // MIPS
        assert!(!assess(&req(10003, (3, 0), (5, 99), &[]), &X51V).usable()); // SH3
        assert!(!assess(&req(2577, (3, 0), (5, 99), &["pocketpc"]), &X51V).usable());
    }

    #[test]
    fn choice_prefers_range_then_processor_then_order() {
        let arm_out = Fit {
            cpu: CpuFit::Arm,
            version_ok: false,
            platform_ok: true,
        };
        let any_in = Fit {
            cpu: CpuFit::Any,
            version_ok: true,
            platform_ok: true,
        };
        let arm_in = Fit {
            cpu: CpuFit::Arm,
            version_ok: true,
            platform_ok: true,
        };
        let mips = Fit {
            cpu: CpuFit::Other,
            version_ok: true,
            platform_ok: true,
        };
        assert_eq!(choose(&[Some(mips), Some(arm_out), Some(any_in)]), Some(2));
        assert_eq!(choose(&[Some(any_in), Some(arm_in)]), Some(1));
        assert_eq!(choose(&[Some(arm_in), Some(arm_in)]), Some(0));
        assert_eq!(choose(&[Some(mips), None]), None);
        assert_eq!(choose(&[None, Some(arm_out)]), Some(1));
    }
}
