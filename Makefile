.PHONY: gpui gpui-check

# macOS no-Node alias. Cross-platform entry: npm run gpui.
gpui:
	sh clarora-gpui/scripts/dev.sh $(ARGS)

gpui-check:
	cargo check --manifest-path clarora-gpui/Cargo.toml --locked
