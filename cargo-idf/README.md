# cargo-idf
 
 A cargo subcommand for the `esp32` platform in conjunction with
 [`esp-idf-sys`](https://crates.io/crates/esp-idf-sys).

## `cargo idf menuconfig`

Opens the menuconfig tool to configure the project configuration interactively.

If the option `--idf-build-info <json-file>` is *not* specified, a `cargo build` will be
performed in the current directory. The needed `esp-idf-build.json` will then be taken
from `<esp-idf-sys out-dir>/esp-idf-build.json`. If this option is specified *its* json
file will be used instead and no build will be performed.

All other options are applied only to the build.

TODO: Add caution about setting optimization options in sdkconfig.

<details>
<summary>
Commands used
</summary>

```console
python esp-idf/tools/kconfig_new/prepare_kconfig_files.py 
    --env-file <out_dir>/build/config.env

python esp-idf/tools/kconfig_new/confgen.py 
    --kconfig esp-idf/Kconfig 
    --sdkconfig-rename esp-idf/sdkconfig.rename 
    --config <out-dir>/sdkconfig 
    --defaults <defaults-file>...
    --env-file <out-dir>/build/config.env 
    --dont-write-deprecated 
    --output config <out-dir>/sdkconfig

python -m menuconfig esp-idf/Kconfig 
    Env variables:
        - KCONFIG_CONFIG=<out-dir>/sdkconfig 
        - <build-dir>/config.env
 ```

</details>

## `cargo idf flash`
## `cargo idf monitor`
## `cargo idf erase-flash`
## `cargo idf size`