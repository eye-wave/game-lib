.PHONY: build

TARGET := target/x86_64-unknown-linux-gnu/release/stupid-game

build:
	cargo build --release
	-strip --strip-unneeded -R .comment -R .eh_frame -R .eh_frame_hdr $(TARGET) 2>/dev/null || true
	-sstrip $(TARGET)
