//! Flash and RAM footprint of ELF executables, and reports comparing builds of them.
//!
//! Intended for CI size tracking: build each firmware at the base commit and at the head
//! of a pull request, [`measure`](Sizes::from_file) both into a [`Measurement`], and
//! render all measurements as one markdown [`Report`]. With the `serde` feature enabled
//! measurements can be serialized, so that they can travel between CI jobs.
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
pub const DEFAULT_RAM_SECTIONS: &[&str] = &[
    ".data", ".bss", ".uninit", ".noinit", ".rwtext", ".rwdata", ".trap",
];

/// The footprint of one ELF executable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

/// The footprint of one build, optionally next to the footprint of its base build.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Measurement {
    /// What was measured, e.g. the example and the chip; the first column of the report.
    pub label: String,
    /// The footprint of the base build, if any.
    pub base: Option<Sizes>,
    /// The footprint of the build.
    pub new: Sizes,
}

/// A region of a build whose growth exceeded the report's threshold.
#[derive(Clone, Debug, PartialEq)]
pub struct Increase {
    /// The label of the measurement.
    pub label: String,
    /// `"FLASH"` or `"RAM"`.
    pub region: &'static str,
    /// Size at the base.
    pub base: u64,
    /// Size at the head.
    pub new: u64,
    /// Growth in percent of the base size.
    pub percent: f64,
}

/// A markdown size report over any number of measurements.
///
/// The report has up to three tables, each listing every measurement:
/// - the regions that grew by more than the `warn` threshold, when one is set;
/// - the FLASH and RAM sizes;
/// - the sizes of all sections.
///
/// Only the first table is expanded; the rest are collapsed.
///
/// Measurements with a base build get base, new, delta and percent columns; the tables
/// show plain sizes when no measurement has a base.
#[derive(Clone, Debug)]
pub struct Report<'a> {
    /// Report title, rendered as a heading when set.
    pub title: Option<&'a str>,
    /// The measurements, listed in this order.
    pub measurements: &'a [Measurement],
    /// Growth in percent above which a region is flagged.
    pub warn: Option<f64>,
}

impl<'a> Report<'a> {
    /// Create a report over `measurements`.
    pub const fn new(measurements: &'a [Measurement]) -> Self {
        Self {
            title: None,
            measurements,
            warn: None,
        }
    }

    /// Set the title.
    pub const fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Flag regions that grew by more than `percent`.
    pub const fn warn(mut self, percent: f64) -> Self {
        self.warn = Some(percent);
        self
    }

    /// The regions that grew by more than the `warn` threshold.
    ///
    /// Empty when there is no threshold or no measurement has a base.
    pub fn increases(&self) -> Vec<Increase> {
        let warn = match self.warn {
            Some(warn) => warn,
            None => return Vec::new(),
        };

        self.measurements
            .iter()
            .flat_map(|m| {
                let regions = m.base.as_ref().map_or(Vec::new(), |base| {
                    vec![
                        ("FLASH", base.flash, m.new.flash),
                        ("RAM", base.ram, m.new.ram),
                    ]
                });

                regions
                    .into_iter()
                    .map(move |(region, base, new)| Increase {
                        label: m.label.clone(),
                        region,
                        base,
                        new,
                        percent: percent(base, new),
                    })
            })
            .filter(|increase| increase.percent > warn)
            .collect()
    }

    /// Render the report as markdown.
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        let has_base = self.measurements.iter().any(|m| m.base.is_some());

        if let Some(title) = self.title {
            let _ = writeln!(out, "#### {title}\n");
        }

        let increases = matches!((has_base, self.warn), (true, Some(_)));

        if let (true, Some(warn)) = (has_base, self.warn) {
            let _ = writeln!(out, "**Increases above {warn}%:**\n");

            let increases = self.increases();
            if increases.is_empty() {
                let _ = writeln!(out, "None.\n");
            } else {
                self.table_header(&mut out, "Region", has_base);
                for i in &increases {
                    self.row(&mut out, &i.label, i.region, Some(i.base), i.new, None);
                }
                let _ = writeln!(out);
            }
        }

        if increases {
            let _ = writeln!(out, "<details><summary><b>Regions</b></summary>\n");
        } else {
            let _ = writeln!(out, "**Regions:**\n");
        }
        self.table_header(&mut out, "Region", has_base);
        for m in self.measurements {
            let base = m.base.as_ref();
            self.row(
                &mut out,
                &m.label,
                "FLASH",
                base.map(|b| b.flash),
                m.new.flash,
                self.warn,
            );
            self.row(
                &mut out,
                &m.label,
                "RAM",
                base.map(|b| b.ram),
                m.new.ram,
                self.warn,
            );
        }

        if increases {
            let _ = writeln!(out, "\n</details>");
        }

        let _ = writeln!(out, "\n<details><summary><b>Sections</b></summary>\n");
        self.table_header(&mut out, "Section", has_base);
        for m in self.measurements {
            let names: BTreeSet<&String> = m
                .base
                .iter()
                .flat_map(|base| base.sections.keys())
                .chain(m.new.sections.keys())
                .collect();

            for name in names {
                let base = m
                    .base
                    .as_ref()
                    .map(|b| b.sections.get(name).copied().unwrap_or(0));
                let new = m.new.sections.get(name).copied().unwrap_or(0);

                self.row(&mut out, &m.label, name, base, new, None);
            }
        }
        let _ = writeln!(out, "\n</details>");

        out
    }

    fn table_header(&self, out: &mut String, what: &str, has_base: bool) {
        if has_base {
            let _ = writeln!(
                out,
                "| Build | {what} | Base | New | Δ | Δ% |\n|---|---|---:|---:|---:|---:|"
            );
        } else {
            let _ = writeln!(out, "| Build | {what} | Size |\n|---|---|---:|");
        }
    }

    fn row(
        &self,
        out: &mut String,
        label: &str,
        name: &str,
        base: Option<u64>,
        new: u64,
        warn: Option<f64>,
    ) {
        let has_base = self.measurements.iter().any(|m| m.base.is_some());

        match base {
            Some(base) => {
                let delta = new as i64 - base as i64;
                let percent = percent(base, new);
                let mark = match warn {
                    Some(warn) if percent > warn => " ⚠️",
                    _ => "",
                };

                let _ = writeln!(
                    out,
                    "| {label} | `{name}` | {base} | {new} | {delta:+} | {percent:+.2}%{mark} |"
                );
            }
            None if has_base => {
                let _ = writeln!(out, "| {label} | `{name}` | - | {new} | - | - |");
            }
            None => {
                let _ = writeln!(out, "| {label} | `{name}` | {new} |");
            }
        }
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

    fn measurement(label: &str, base: Option<Sizes>, new: Sizes) -> Measurement {
        Measurement {
            label: label.to_string(),
            base,
            new,
        }
    }

    #[test]
    fn diff_report_has_three_tables_over_all_measurements() {
        let ms = [
            measurement(
                "riscv",
                Some(sizes(
                    1000,
                    200,
                    &[(".text", 800), (".rodata", 200), (".bss", 200)],
                )),
                sizes(
                    1010,
                    190,
                    &[(".text", 810), (".rodata", 200), (".bss", 190)],
                ),
            ),
            measurement(
                "arm",
                Some(sizes(500, 100, &[(".text", 500), (".bss", 100)])),
                sizes(500, 100, &[(".text", 500), (".bss", 100)]),
            ),
        ];

        let report = Report::new(&ms).title("fw").warn(0.5);
        let md = report.markdown();

        assert!(md.starts_with("#### fw\n"), "{md}");
        assert!(md.contains("**Increases above 0.5%:**"), "{md}");
        assert!(
            md.contains("| riscv | `FLASH` | 1000 | 1010 | +10 | +1.00% |"),
            "{md}"
        );
        assert!(
            md.contains("| riscv | `FLASH` | 1000 | 1010 | +10 | +1.00% ⚠️ |"),
            "{md}"
        );
        assert!(
            md.contains("| riscv | `RAM` | 200 | 190 | -10 | -5.00% |"),
            "{md}"
        );
        assert!(
            md.contains("| arm | `FLASH` | 500 | 500 | +0 | +0.00% |"),
            "{md}"
        );
        assert!(
            md.contains("| riscv | `.rodata` | 200 | 200 | +0 | +0.00% |"),
            "{md}"
        );
        assert!(
            md.contains("| arm | `.bss` | 100 | 100 | +0 | +0.00% |"),
            "{md}"
        );
        assert!(
            md.contains("<details><summary><b>Regions</b></summary>"),
            "{md}"
        );

        let increases = report.increases();
        assert_eq!(increases.len(), 1);
        assert_eq!(increases[0].label, "riscv");
        assert_eq!(increases[0].region, "FLASH");
    }

    #[test]
    fn no_increases_says_so() {
        let ms = [measurement(
            "arm",
            Some(sizes(500, 100, &[])),
            sizes(500, 100, &[]),
        )];
        let md = Report::new(&ms).warn(0.5).markdown();
        assert!(md.contains("**Increases above 0.5%:**\n\nNone.\n"), "{md}");
    }

    #[test]
    fn plain_report_lists_sizes_only() {
        let ms = [measurement(
            "arm",
            None,
            sizes(1000, 200, &[(".text", 800), (".bss", 200)]),
        )];
        let md = Report::new(&ms).warn(0.0).markdown();
        assert!(!md.contains("Increases"), "{md}");
        assert!(md.contains("| Build | Region | Size |"), "{md}");
        assert!(md.contains("**Regions:**"), "{md}");
        assert!(md.contains("| arm | `FLASH` | 1000 |"), "{md}");
        assert!(md.contains("| arm | `.text` | 800 |"), "{md}");
        assert!(Report::new(&ms).warn(0.0).increases().is_empty());
    }

    #[test]
    fn measurement_without_base_next_to_one_with_base() {
        let ms = [
            measurement("a", Some(sizes(10, 1, &[])), sizes(10, 1, &[])),
            measurement("b", None, sizes(20, 2, &[])),
        ];
        let md = Report::new(&ms).markdown();
        assert!(md.contains("| b | `FLASH` | - | 20 | - | - |"), "{md}");
    }

    #[test]
    fn measures_the_test_binary() {
        let sizes =
            Sizes::from_file(std::env::current_exe().unwrap(), DEFAULT_RAM_SECTIONS).unwrap();
        assert!(sizes.flash > 0);
        assert!(sizes.sections.contains_key(".text"));
    }
}
