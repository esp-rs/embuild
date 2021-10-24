# A linker proxy

A simple tool to forward linker arguments to the actual linker executable also given as an argument to `ldproxy`.

*Currently only gcc [linker
flavor](https://doc.rust-lang.org/rustc/codegen-options/index.html#linker-flavor) is
supported.*

## Special arguments

These arguments are only used by `ldproxy` and not forwarded to the proxied linker.

- `--ldproxy-linker=<path>`, `--ldproxy-linker <path>`

    **required**

    Tells `ldproxy` the path to the linker. If multiple `--ld-proxy` arguments are found
    only the last will be used.
    
- `--ldproxy-cwd=<path>`, `--ldproxy-cwd <path>`

    **optional**

    Tells `ldproxy` the current working directory to use when it invokes the linker.
