# cargo-elfsize

Prints the flash and RAM footprint of an ELF executable as a markdown table, or the
difference between two builds of it. Meant for tracking firmware size in CI: build the
firmware at the base commit and at the head of a pull request, then diff the two.

```
cargo install cargo-elfsize
cargo elfsize --title "esp32c6 client" --warn 0.2 base.elf new.elf
```

produces

```
#### esp32c6 client

| Region | Base | New | Δ | Δ% |
|---|---:|---:|---:|---:|
| `FLASH` | 1066516 | 1080804 | +14288 | +1.34% ⚠️ |
| `RAM` | 207608 | 177064 | -30544 | -14.71% |

<details><summary>Sections</summary>
...
</details>
::warning title=esp32c6 client::FLASH grew by 1.34% (1066516 -> 1080804 bytes)
```

The last line is a [GitHub Actions annotation](https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/workflow-commands-for-github-actions)
and only appears with `--warn`. Redirect the output to `$GITHUB_STEP_SUMMARY` to get the
table on the job summary page.

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

- `--title <TITLE>` - report title, default `Size report`
- `--warn <PERCENT>` - emit a `::warning::` annotation for each region that grew by more than this
- `--ram <PREFIX,...>` - section name prefixes counted as RAM

The same reports are available programmatically from the `embuild::elfsize` module
(feature `elf`).
