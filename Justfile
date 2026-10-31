set quiet
set shell := ["bash", "-cue"]

target := if os_family() == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
}

exe := if os_family() == "windows" { ".exe" } else { "" }
extra := if os_family() == "windows" {
    "-Zbuild-std-features=compiler-builtins-mem"
} else {
    ""
}

build app="example-3d":
    cargo build --release -p {{quote(app)}} --target {{target}} {{extra}}
    if [ "{{os_family()}}" = "windows" ]; then \
        llvm-strip --strip-all "target/{{target}}/release/{{app}}{{exe}}" || true; \
        if [ -n "${PFX:-}" ]; then \
            MSYS2_ARG_CONV_EXCL='*' signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f "$PFX" /p "${PFX_PASS:-}" "target/{{target}}/release/{{app}}{{exe}}"; \
        fi; \
    else \
        strip --strip-all \
            -R .comment \
            -R .eh_frame \
            -R .eh_frame_hdr \
            "target/{{target}}/release/{{app}}{{exe}}" 2>/dev/null || true; \
        sstrip "target/{{target}}/release/{{app}}{{exe}}" || true; \
    fi

app name:
    just build "{{name}}"

bundle app="" dir="target/bundle":
    mkdir -p "{{dir}}"
    for path in apps/*/; do \
        name="${path%/}"; name="${name##*/}"; \
        if [ -n "{{app}}" ] && [ "$name" != "{{app}}" ]; then continue; fi; \
        just build "$name"; \
        cp "target/{{target}}/release/$name{{exe}}" "{{dir}}/"; \
    done
    echo "Bundled apps:"
    stat -c '%n %s' target/bundle/* | awk '{printf "%s %.2f KiB\n", $1, $2/1024}'
    cp -r assets target/bundle/assets
