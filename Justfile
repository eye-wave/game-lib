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

generate-error-ref:
    RUSTFLAGS="-Cdebuginfo=0" cargo run -p errors --features generate-error-docs -- target/bundle/error-table.html

bundle app="" dir="target/bundle":
    rm -rf target/bundle
    mkdir -p "{{dir}}"
    for path in apps/*/; do \
        name="${path%/}"; name="${name##*/}"; \
        if [ -n "{{app}}" ] && [ "$name" != "{{app}}" ]; then continue; fi; \
        just build "$name"; \
        cp "target/{{target}}/release/$name{{exe}}" "{{dir}}/"; \
    done

    echo "Bundled apps:"
    find target/bundle -maxdepth 1 -type f -printf '%s %p\n' | sort -nr | awk '{printf "%.2f KiB %s\n", $1/1024, $2}'

    just generate-error-ref
    cp -r assets target/bundle/assets
