set quiet
set shell := ["bash", "-cu"]

target := if os_family() == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
}

exe := if os_family() == "windows" { ".exe" } else { "" }

build app="example-3d":
    cargo build --release -p "{{app}}" --target {{target}}
    if [ "{{os_family()}}" = "windows" ]; then \
        llvm-strip --strip-all "target/{{target}}/release/{{app}}{{exe}}" || true; \
        signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f "$PFX" /p "$PFX_PASS" "target/{{target}}/release/{{app}}{{exe}}"; \
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
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{dir}}"

    for path in apps/*/; do
        name="${path%/}"
        name="${name##*/}"

        if [ -n "{{app}}" ] && [ "$name" != "{{app}}" ]; then
            continue
        fi

        just build "$name"
        cp "target/{{target}}/release/$name{{exe}}" "{{dir}}/"
    done

    echo "Bundled apps:"
    ls -lh "{{dir}}"
