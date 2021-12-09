# cargo-idf
 - `esp-idf` dir
 - `out-dir`
 - `esp-idf build` dir
 - esp-idf `python`


## `cargo idf menuconfig`

- `python esp-idf/tools/kconfig_new/prepare_kconfig_files.py --env-file
  <out_dir>/build/config.env` (implement in rust)

python esp-idf/tools/kconfig_new/confgen.py --kconfig esp-idf/Kconfig --sdkconfig-rename
esp-idf/sdkconfig.rename --config <out-dir>/sdkconfig --defaults ... --env-file <out-dir>/build/config.env
--env IDF_TARGET=esp32c3 --env IDF_ENV_FPGA= --dont-write-deprecated --output config <out-dir>/sdkconfig

python esp-idf/tools/check_term.py 

- COMPONENT_KCONFIGS_SOURCE_FILE=<out-dir>/build/kconfigs.in 
- COMPONENT_KCONFIGS_PROJBUILD_SOURCE_FILE=<out-dir>/build/kconfigs_projbuild.in 
- IDF_CMAKE=y 
- KCONFIG_CONFIG=<out-dir>/sdkconfig 
- IDF_TARGET=esp32c3 IDF_ENV_FPGA= 
python -m menuconfig esp-idf/Kconfig 

python esp-idf/tools/kconfig_new/confgen.py --kconfig esp-idf/Kconfig --sdkconfig-rename esp-idf/sdkconfig.rename --config <out-dir>/sdkconfig --defaults ... --env-file <out-dir>/build/config.env --env IDF_TARGET=esp32c3 --env IDF_ENV_FPGA= --output config <out-file>
 ```

## `cargo idf flash`
## `cargo idf monitor`
## `cargo idf erase-flash`
## `cargo idf size`