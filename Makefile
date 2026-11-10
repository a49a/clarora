.PHONY: gpui gpui-check

# Native desktop development entry point; npm run gpui remains an alias.
gpui:
	sh clarora-gpui/scripts/dev.sh $(ARGS)

gpui-check:
	cargo check --manifest-path clarora-gpui/Cargo.toml --locked
