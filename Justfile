set quiet
set shell := ["bash", "-cue"]

target := if os_family() == "windows" { "x86_64-pc-windows-msvc" } else { "x86_64-unknown-linux-gnu" }

exe := if os_family() == "windows" { ".exe" } else { "" }
extra := if os_family() == "windows" { "-Zbuild-std-features=compiler-builtins-mem" } else { "" }

# Replaces the PE stub text on windows.
export STUB := " (https://github.com/eye-wave/game-lib)"
webview2 := "1.0.3240.44"

build app="example-3d":
    cargo build --release -p {{ quote(app) }} --target {{ target }} {{ extra }}
    if [ "{{ os_family() }}" = "windows" ]; then \
        llvm-strip --strip-all "target/{{ target }}/release/{{ app }}{{ exe }}" || true; \
        [ ${#STUB} -le 39 ]; \
        perl -0777 -pi -e 'BEGIN { $s = sprintf("%-39s", $ENV{STUB}) } s/This program cannot be run in DOS mode\./$s/' \
            "target/{{ target }}/release/{{ app }}{{ exe }}"; \
        if [ -n "${PFX:-}" ]; then \
            MSYS2_ARG_CONV_EXCL='*' signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f "$PFX" /p "${PFX_PASS:-}" "target/{{ target }}/release/{{ app }}{{ exe }}"; \
        fi; \
    else \
        strip --strip-all \
            -R .comment \
            -R .eh_frame \
            -R .eh_frame_hdr \
            "target/{{ target }}/release/{{ app }}{{ exe }}" 2>/dev/null || true; \
        sstrip "target/{{ target }}/release/{{ app }}{{ exe }}" || true; \
    fi

app name:
    just build "{{ name }}"

generate-error-ref:
    RUSTFLAGS="-Cdebuginfo=0" cargo run -p errors --features generate-error-docs -- target/bundle/error-table.html

bundle app="" dir="target/bundle":
    rm -rf target/bundle
    mkdir -p "{{ dir }}"
    for path in apps/*/; do \
        name="${path%/}"; name="${name##*/}"; \
        if [ -n "{{ app }}" ] && [ "$name" != "{{ app }}" ]; then continue; fi; \
        echo "$name"; \
        just build "$name"; \
        cp "target/{{ target }}/release/$name{{ exe }}" "{{ dir }}/"; \
    done

    if [ "{{ os_family() }}" = "windows" ]; then \
        curl -fsSL -o target/webview2.zip "https://www.nuget.org/api/v2/package/Microsoft.Web.WebView2/{{ webview2 }}"; \
        "$SYSTEMROOT/System32/tar.exe" -xf target/webview2.zip -C "{{ dir }}" --strip-components=3 build/native/x64/WebView2Loader.dll; \
    fi

    echo "Bundled apps:"
    find target/bundle -maxdepth 1 -type f -printf '%s %p\n' | sort -nr | awk '{printf "%.2f KiB %s\n", $1/1024, $2}'

    just generate-error-ref
    cp -r assets target/bundle/assets
