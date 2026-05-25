# Build the masterbus C demos.
#
# Local (native) build is the default — it links against the host cdylib:
#   make                       # build all demos for the local host
#   make run [IFACE=can0]      # build + run mb_enumerate locally (needs SocketCAN)
#   make clean
#
# Remote build cross-compiles for another machine (default: a Raspberry Pi) and
# deploys over ssh. Override REMOTE_HOST / RUST_TARGET / CROSS_CC as needed:
#   make remote                # cross-compile into ./remote/ + build the cdylib
#   make deploy                # remote + scp demos and the .so to REMOTE_HOST
#   make run-remote            # deploy + run mb_enumerate on REMOTE_HOST
#   make remote REMOTE_HOST=pi@boat RUST_TARGET=armv7-unknown-linux-gnueabihf \
#               CROSS_CC=arm-linux-gnueabihf-gcc

WS_ROOT := ../../..
INC     := ../include
DEMOS   := mb_enumerate mb_set_value mb_get_value

# ---- local (native) build -------------------------------------------------
CC       ?= cc
CFLAGS   ?= -std=c11 -O2 -Wall -Wextra -D_POSIX_C_SOURCE=200112L -I$(INC) -I.
HOST_LIB := $(WS_ROOT)/target/release

UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Linux)
    SYS_LIBS := -lpthread -ldl -lm
else
    SYS_LIBS := -lm
endif
LDFLAGS := -L$(HOST_LIB) -lmasterbus_ffi $(SYS_LIBS)

IFACE ?= can0

# ---- remote (cross) build -------------------------------------------------
REMOTE_HOST ?= root@merrimac-pi
REMOTE_DIR  ?= ~/masterbus-demo
RUST_TARGET ?= aarch64-unknown-linux-gnu
CROSS_CC    ?= aarch64-linux-gnu-gcc
CAN         ?= can-masterbus

REMOTE_BUILD := remote-bin
REMOTE_BINS  := $(addprefix $(REMOTE_BUILD)/,$(DEMOS))
REMOTE_LIB   := $(WS_ROOT)/target/$(RUST_TARGET)/release/libmasterbus_ffi.so
CROSS_CFLAGS := -std=c11 -Os -fPIE -pie -Wall -Wextra -D_POSIX_C_SOURCE=200112L -I$(INC) -I.
CROSS_LDFLAGS := -L$(WS_ROOT)/target/$(RUST_TARGET)/release -lmasterbus_ffi -lm -lpthread -ldl

.PHONY: all lib run clean remote remote-lib deploy run-remote

# ---- default: local -------------------------------------------------------
all: $(DEMOS)

lib:
	cd $(WS_ROOT) && cargo build -p masterbus-ffi --release

$(DEMOS): %: %.c mb_print.h $(INC)/masterbus.h lib
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

run: all
	LD_LIBRARY_PATH=$(HOST_LIB) ./mb_enumerate $(IFACE)

# ---- remote ---------------------------------------------------------------
remote: $(REMOTE_BINS)

remote-lib:
	cd $(WS_ROOT) && cargo build -p masterbus-ffi --release --target $(RUST_TARGET)

$(REMOTE_BUILD):
	mkdir -p $(REMOTE_BUILD)

$(REMOTE_BINS): $(REMOTE_BUILD)/%: %.c mb_print.h $(INC)/masterbus.h remote-lib | $(REMOTE_BUILD)
	$(CROSS_CC) $(CROSS_CFLAGS) $< $(CROSS_LDFLAGS) -o $@

deploy: remote
	ssh $(REMOTE_HOST) mkdir -p $(REMOTE_DIR)
	scp $(REMOTE_BINS) $(REMOTE_LIB) $(REMOTE_HOST):$(REMOTE_DIR)/

run-remote: deploy
	ssh $(REMOTE_HOST) "cd $(REMOTE_DIR) && LD_LIBRARY_PATH=. ./mb_enumerate $(CAN)"

clean:
	rm -f $(DEMOS)
	rm -rf $(REMOTE_BUILD)
