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
<summary>Commands used</summary>

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


<details>
<summary>Commands used</summary>

```console
cmake.exe 
    -D IDF_PATH="..." 
    -D SERIAL_TOOL="python esp-idf-v4.3.1/components/esptool_py/esptool/esptool.py --chip esp32c3"
    -D SERIAL_TOOL_ARGS="--before=default_reset --after=hard_reset write_flash @flash_args"
    -D WORKING_DIRECTORY=<out-dir>/build
    -P esp-idf-v4.3.1/components/esptool_py/run_serial_tool.cmake
```

</details>

## `cargo idf monitor`
## `cargo idf erase-flash`

<details>
<summary>Commands used</summary>

```console
  COMMAND = cmd.exe /C "cd /D C:\Users\n3xed\.espressif\esp-idf-v4.3.1\components\esptool_py && C:\Users\n3xed\.espressif\tools\cmake\3.20.3\bin\cmake.exe -D IDF_PATH="C:/Users/n3xed/.espressif/esp-idf-v4.3.1" -D SERIAL_TOOL="python C:/Users/n3xed/.espressif/esp-idf-v4.3.1/components/esptool_py/esptool/esptool.py --chip esp32c3" -D SERIAL_TOOL_ARGS="erase_flash" -P run_serial_tool.cmake"
```

</details>

## `cargo idf size`