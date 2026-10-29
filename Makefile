.PHONY: build $(notdir $(wildcard apps/*))

APP ?= example-3d

ifeq ($(OS),Windows_NT)
TARGET := x86_64-pc-windows-msvc
BIN := target/$(TARGET)/release/$(APP).exe
STRIP := llvm-strip
EXTRA := -Zbuild-std-features=compiler-builtins-mem
else
TARGET := x86_64-unknown-linux-gnu
BIN := target/$(TARGET)/release/$(APP)
STRIP := strip
endif

build:
	cargo build --release -p $(APP) --target $(TARGET) $(EXTRA)
ifeq ($(OS),Windows_NT)
	-$(STRIP) --strip-all $(BIN)
	-signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f "$(PFX)" /p "$(PFX_PASS)" $(BIN)
else
	-strip --strip-all \
		-R .comment \
		-R .eh_frame \
		-R .eh_frame_hdr \
		$(BIN) 2>/dev/null || true
	-sstrip $(BIN)
endif

$(notdir $(wildcard apps/*)):
	$(MAKE) build APP=$@
