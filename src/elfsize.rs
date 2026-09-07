//! Flash and RAM footprint of ELF executables, and reports comparing two of them.
//!
//! Intended for CI size tracking: build the same firmware at the base commit and at the
//! head of a pull request, then render the difference as a markdown table.
//!
//! **FLASH** is the sum of the file-backed bytes of all `PT_LOAD` segments. That is
//! exactly the image that ends up in flash, so it needs no knowledge of section names and
//! includes RAM-resident code and data that are copied out of flash at startup.
//!
//! **RAM** is the sum of the allocated sections whose names start with one of the
//! configured prefixes. Fixed-size reservations like `.stack` and `.heap` are deliberately
//! not in the default list: some linker scripts (esp-hal for one) size the stack as
//! whatever RAM is left, so counting it would make every RAM delta zero.
//!
//! # Why not `cargo size`?
//!
//! `cargo size` (from `cargo-binutils`) builds the crate and runs `llvm-size` on the
//! artifact. It differs from this module in what it needs, what it measures and what it
//! outputs:
//!
//! - It reports one build; it cannot diff two builds, flag a threshold, or produce
//!   markdown. That is most of what CI size tracking needs and most of this module.
//! - Its Berkeley `text`/`data`/`bss` figures are heuristics over section flags, not
//!   flash and RAM occupancy. `text` is every allocated read-only section, so it includes
//!   NOLOAD gaps and reservations that never reach flash, and it files RAM-resident code
//!   (`.rwtext` on esp-hal) under `text` rather than RAM. On an esp-hal binary this
//!   overstates the flash image by several hundred KB. `llvm-size -A` lists the raw
//!   sections instead, but then the grouping logic is needed anyway.
//! - It requires the `llvm-tools` rustup component in the active toolchain. The
//!   Xtensa toolchain installed by `espup` does not ship `llvm-size`, so it cannot run on
//!   ESP32 and ESP32-S3 builds at all. This module parses the ELF itself and needs
//!   nothing from the toolchain.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Error, Result};
use xmas_elf::program::Type;
use xmas_elf::sections::SHF_ALLOC;
use xmas_elf::ElfFile;

/// Section name prefixes counted as RAM by default.
///
/// Covers the cortex-m and esp-hal linker scripts.
pub const DEFAULT_RAM_SECTIONS: &[&str] =
    &[".data", ".bss", ".rwtext", ".rwdata", ".noinit", ".trap"];

/// The footprint of one ELF executable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sizes {
    /// Bytes occupied in flash.
    pub flash: u64,
    /// Bytes occupied in RAM.
    pub ram: u64,
    /// Size of every allocated, non-empty section, by name.
    pub sections: BTreeMap<String, u64>,
}

impl Sizes {
    /// Measure the ELF file at `path`, counting sections whose names start with one of
    /// the `ram_sections` prefixes as RAM.
    pub fn from_file(path: impl AsRef<Path>, ram_sections: &[&str]) -> Result<Self> {
        let path = path.as_ref();
        let data = fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;

        Self::from_elf(&data, ram_sections)
            .with_context(|| format!("Cannot measure {}", path.display()))
    }

    /// Measure the ELF image `data`, counting sections whose names start with one of
    /// the `ram_sections` prefixes as RAM.
    pub fn from_elf(data: &[u8], ram_sections: &[&str]) -> Result<Self> {
        let elf = ElfFile::new(data).map_err(Error::msg)?;

        let mut sizes = Self::default();

        for segment in elf.program_iter() {
            if segment.get_type() == Ok(Type::Load) {
                sizes.flash += segment.file_size();
            }
        }

        for section in elf.section_iter() {
            if section.flags() & SHF_ALLOC == 0 || section.size() == 0 {
                continue;
            }

            let name = section.get_name(&elf).map_err(Error::msg)?;

            if ram_sections.iter().any(|prefix| name.starts_with(prefix)) {
                sizes.ram += section.size();
            }

            sizes.sections.insert(name.to_string(), section.size());
        }

        Ok(sizes)
    }
}

/// A region whose growth exceeded the report's threshold.
#[derive(Clone, Debug, PartialEq)]
pub struct Increase {
    /// `"FLASH"` or `"RAM"`.
    pub region: &'static str,
    /// Size at the base.
    pub base: u64,
    /// Size at the head.
    pub new: u64,
    /// Growth in percent of the base size.
    pub percent: f64,
}

/// A size report for one ELF executable, optionally compared against a base build.
#[derive(Clone, Debug)]
pub struct Report<'a> {
    /// Report title.
    pub title: &'a str,
    /// The footprint of the base build, if any.
    pub base: Option<&'a Sizes>,
    /// The footprint of the build being reported on.
    pub new: &'a Sizes,
    /// Growth in percent above which a region is flagged.
    pub warn: Option<f64>,
}

impl<'a> Report<'a> {
    /// Create a report of `new` alone.
    pub const fn new(title: &'a str, new: &'a Sizes) -> Self {
        Self {
            title,
            base: None,
            new,
            warn: None,
        }
    }

    /// Compare against `base`.
    pub const fn base(mut self, base: &'a Sizes) -> Self {
        self.base = Some(base);
        self
    }

    /// Flag regions that grew by more than `percent`.
    pub const fn warn(mut self, percent: f64) -> Self {
        self.warn = Some(percent);
        self
    }

    /// The regions that grew by more than the `warn` threshold.
    ///
    /// Empty when there is no base or no threshold.
    pub fn increases(&self) -> Vec<Increase> {
        let (base, warn) = match (self.base, self.warn) {
            (Some(base), Some(warn)) => (base, warn),
            _ => return Vec::new(),
        };

        [
            ("FLASH", base.flash, self.new.flash),
            ("RAM", base.ram, self.new.ram),
        ]
        .into_iter()
        .map(|(region, base, new)| Increase {
            region,
            base,
            new,
            percent: percent(base, new),
        })
        .filter(|increase| increase.percent > warn)
        .collect()
    }

    /// Render the report as markdown: a FLASH/RAM table followed by a collapsed table of
    /// the sections that differ (or of all sections, when there is no base).
    pub fn markdown(&self) -> String {
        let mut out = String::new();

        let _ = writeln!(out, "#### {}\n", self.title);
        self.table_header(&mut out, "Region");
        self.row(
            &mut out,
            "FLASH",
            self.base.map(|base| base.flash),
            self.new.flash,
            self.warn,
        );
        self.row(
            &mut out,
            "RAM",
            self.base.map(|base| base.ram),
            self.new.ram,
            self.warn,
        );

        let _ = writeln!(out, "\n<details><summary>Sections</summary>\n");
        self.table_header(&mut out, "Section");

        let names: BTreeSet<&String> = self
            .base
            .into_iter()
            .flat_map(|base| base.sections.keys())
            .chain(self.new.sections.keys())
            .collect();

        for name in names {
            let base = self
                .base
                .map(|base| base.sections.get(name).copied().unwrap_or(0));
            let new = self.new.sections.get(name).copied().unwrap_or(0);

            if base != Some(new) {
                self.row(&mut out, name, base, new, None);
            }
        }

        let _ = writeln!(out, "\n</details>");

        out
    }

    fn table_header(&self, out: &mut String, what: &str) {
        if self.base.is_some() {
            let _ = writeln!(
                out,
                "| {what} | Base | New | Δ | Δ% |\n|---|---:|---:|---:|---:|"
            );
        } else {
            let _ = writeln!(out, "| {what} | Size |\n|---|---:|");
        }
    }

    fn row(&self, out: &mut String, name: &str, base: Option<u64>, new: u64, warn: Option<f64>) {
        let base = match base {
            Some(base) => base,
            None => {
                let _ = writeln!(out, "| `{name}` | {new} |");
                return;
            }
        };

        let delta = new as i64 - base as i64;
        let percent = percent(base, new);
        let mark = match warn {
            Some(warn) if percent > warn => " ⚠️",
            _ => "",
        };

        let _ = writeln!(
            out,
            "| `{name}` | {base} | {new} | {delta:+} | {percent:+.2}%{mark} |"
        );
    }
}

fn percent(base: u64, new: u64) -> f64 {
    if base == 0 {
        0.0
    } else {
        (new as f64 - base as f64) * 100.0 / base as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(flash: u64, ram: u64, sections: &[(&str, u64)]) -> Sizes {
        Sizes {
            flash,
            ram,
            sections: sections
                .iter()
                .map(|(name, size)| (name.to_string(), *size))
                .collect(),
        }
    }

    #[test]
    fn diff_lists_only_changed_sections_and_flags_increases() {
        let base = sizes(
            1000,
            200,
            &[(".text", 800), (".rodata", 200), (".bss", 200)],
        );
        let new = sizes(
            1010,
            190,
            &[(".text", 810), (".rodata", 200), (".bss", 190)],
        );

        let report = Report::new("fw", &new).base(&base).warn(0.5);

        let md = report.markdown();
        assert!(
            md.contains("| `FLASH` | 1000 | 1010 | +10 | +1.00% ⚠️ |"),
            "{md}"
        );
        assert!(md.contains("| `RAM` | 200 | 190 | -10 | -5.00% |"), "{md}");
        assert!(
            md.contains("| `.text` | 800 | 810 | +10 | +1.25% |"),
            "{md}"
        );
        assert!(md.contains("| `.bss` | 200 | 190 | -10 | -5.00% |"), "{md}");
        assert!(!md.contains(".rodata"), "{md}");

        let increases = report.increases();
        assert_eq!(increases.len(), 1);
        assert_eq!(increases[0].region, "FLASH");
        assert_eq!(increases[0].base, 1000);
        assert_eq!(increases[0].new, 1010);
    }

    #[test]
    fn single_build_lists_all_sections() {
        let new = sizes(1000, 200, &[(".text", 800), (".bss", 200)]);

        let md = Report::new("fw", &new).markdown();
        assert!(md.contains("| `FLASH` | 1000 |"), "{md}");
        assert!(md.contains("| `.text` | 800 |"), "{md}");
        assert!(md.contains("| `.bss` | 200 |"), "{md}");
        assert!(Report::new("fw", &new).warn(0.0).increases().is_empty());
    }

    #[test]
    fn measures_the_test_binary() {
        let sizes =
            Sizes::from_file(std::env::current_exe().unwrap(), DEFAULT_RAM_SECTIONS).unwrap();
        assert!(sizes.flash > 0);
        assert!(sizes.sections.contains_key(".text"));
    }
}
