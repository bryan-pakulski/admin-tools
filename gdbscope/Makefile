.PHONY: build build-static clean test

BINARY_NAME := gdbscope
DOCKER_IMAGE := gdbscope-builder
STATIC_DIR := target/static

build:
	cargo build --release

build-static:
	docker build -t $(DOCKER_IMAGE) .
	mkdir -p $(STATIC_DIR)
	@docker rm -f gdbscope-extract 2>/dev/null || true
	docker create --name gdbscope-extract $(DOCKER_IMAGE) /gdbscope
	docker cp gdbscope-extract:/gdbscope $(STATIC_DIR)/$(BINARY_NAME)
	docker rm gdbscope-extract
	@echo ""
	@file $(STATIC_DIR)/$(BINARY_NAME)
	@ls -lh $(STATIC_DIR)/$(BINARY_NAME)
	@echo ""
	@echo "Static binary: $(STATIC_DIR)/$(BINARY_NAME)"

test:
	cargo test

clean:
	cargo clean
	-docker rmi $(DOCKER_IMAGE) 2>/dev/null
	rm -rf $(STATIC_DIR)
