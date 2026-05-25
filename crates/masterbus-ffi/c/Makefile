# Cross-compile the masterbus C demos for the Pi and deploy them.
#
#   make            # build all demos (cross-compiles the cdylib too)
#   make deploy     # scp demos + libmasterbus_ffi.so to the Pi
#   make run        # deploy and run enumerate_bus on the Pi
#   make clean

RUST_TARGET := aarch64-unknown-linux-gnu
WS_ROOT     := ../../..
LIB_DIR     := $(WS_ROOT)/target/$(RUST_TARGET)/release
LIB_SO      := $(LIB_DIR)/libmasterbus_ffi.so
INC         := ../include

CC      := aarch64-linux-gnu-gcc
CFLAGS  := -std=c11 -Os -fPIE -pie -Wall -Wextra -D_POSIX_C_SOURCE=200112L -I$(INC) -I.
LDFLAGS := -L$(LIB_DIR) -lmasterbus_ffi -lm -lpthread -ldl

PI_HOST := root@merrimac-pi
PI_DIR  := ~/masterbus-demo
CAN     := can-masterbus

DEMOS := enumerate_bus mb_switch mb_value

.PHONY: all lib deploy run clean

all: $(DEMOS)

lib:
	cd $(WS_ROOT) && cargo build -p masterbus-ffi --release --target $(RUST_TARGET)

$(DEMOS): %: %.c mb_print.h $(INC)/masterbus.h lib
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

deploy: $(DEMOS)
	ssh $(PI_HOST) mkdir -p $(PI_DIR)
	scp $(DEMOS) $(LIB_SO) $(PI_HOST):$(PI_DIR)/

run: deploy
	ssh $(PI_HOST) "cd $(PI_DIR) && LD_LIBRARY_PATH=. ./enumerate_bus $(CAN)"

clean:
	rm -f $(DEMOS)
