.PHONY: build

ifeq ($(OS),Windows_NT)
TARGET := x86_64-pc-windows-msvc
BIN := target/$(TARGET)/release/stupid-game.exe
EXTRA := -Zbuild-std-features=compiler-builtins-mem
else
TARGET := x86_64-unknown-linux-gnu
BIN := target/$(TARGET)/release/stupid-game
endif

build:
	cargo build --release --target $(TARGET) $(EXTRA)
ifneq ($(OS),Windows_NT)
	-strip --strip-unneeded -R .comment -R .eh_frame -R .eh_frame_hdr $(BIN) 2>/dev/null || true
	-sstrip $(BIN)
endif
