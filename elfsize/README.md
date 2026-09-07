# elfsize

Prints the flash and RAM footprint of an ELF executable as a markdown table, or the
difference between two builds of it. Meant for tracking firmware size in CI: build the
firmware at the base commit and at the head of a pull request, then diff the two.

```
cargo install elfsize

# In every build job: measure the head build, or the base and the head builds
elfsize measure --label "basic_udp on esp32c6" --output esp32c6.json base.elf head.elf

# In a final job: render all measurements as one report
elfsize report --warn 0.2 esp32c6.json nrf52840.json
```

produces

```
**Increases above 0.2%:**

| Build | Region | Base | New | Δ | Δ% |
|---|---|---:|---:|---:|---:|
| basic_udp on esp32c6 | `FLASH` | 319844 | 320668 | +824 | +0.26% |

<details><summary><b>Regions</b></summary>

| Build | Region | Base | New | Δ | Δ% |
|---|---|---:|---:|---:|---:|
| basic_udp on esp32c6 | `FLASH` | 319844 | 320668 | +824 | +0.26% ⚠️ |
| basic_udp on esp32c6 | `RAM` | 48272 | 48280 | +8 | +0.02% |
| basic_udp on nrf52840 | `FLASH` | 183712 | 183712 | +0 | +0.00% |
| basic_udp on nrf52840 | `RAM` | 37276 | 37276 | +0 | +0.00% |

</details>
<details><summary><b>Sections</b></summary>
...
</details>
::warning title=basic_udp on esp32c6::FLASH grew by 0.26% (319844 -> 320668 bytes)
```

Every table lists every measurement, so the same change can be compared across chips.
Only the first table is expanded; without `--warn` that is the regions table.
The last line is a [GitHub Actions annotation](https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/workflow-commands-for-github-actions)
and only appears with `--warn`. Redirect the output to `$GITHUB_STEP_SUMMARY` to get the
tables on the job summary page. Measurements are plain JSON, so they can be uploaded as
artifacts by parallel build jobs and collected by a final report job.

## What is measured

**FLASH** is the sum of the file-backed bytes of all `PT_LOAD` segments: exactly the image
that ends up in flash, regardless of how the linker script names its sections. It includes
the initial values of `.data` and any RAM-resident code that is copied out of flash at startup.

**RAM** is the sum of the allocated sections whose names start with one of the `--ram`
prefixes. The default list, `.data,.bss,.uninit,.noinit,.rwtext,.rwdata,.trap`, covers the
cortex-m and esp-hal linker scripts. Fixed-size reservations like `.stack` and `.heap` are
deliberately not counted: some linker scripts size the stack as whatever RAM is left, so
counting it would make every RAM delta zero.

## Options

`measure`:
- `--label <LABEL>` - the build's name in the report, default the ELF file name
- `--ram <PREFIX,...>` - section name prefixes counted as RAM
- `--output <FILE>` - write the JSON measurement there rather than to stdout

`report`:
- `--title <TITLE>` - a heading above the tables
- `--warn <PERCENT>` - list the regions that grew by more than this first, and emit a `::warning::` annotation for each

The same reports are available programmatically from the `embuild::elfsize` module
(feature `elf`).

The tool was briefly published as `cargo-elfsize`; that crate is yanked, as the tool does
not build anything and should never have been a cargo subcommand.
