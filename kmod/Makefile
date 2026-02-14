# SPDX-License-Identifier: GPL-2.0
#
# KSVC kernel module Makefile
# Target: Ubuntu 24.04 / Linux 6.8+
#
# Build:     make
# Install:   sudo insmod ksvc.ko
# Unload:    sudo rmmod ksvc
# Clean:     make clean
# DKMS:      make dkms-install

KDIR ?= /lib/modules/$(shell uname -r)/build
MODULE_NAME := ksvc

obj-m := $(MODULE_NAME).o
$(MODULE_NAME)-y := ksvc_main.o ksvc_ring.o ksvc_shared.o

# Extra CFLAGS for warnings
ccflags-y := -Wall -Werror -Wextra -Wno-unused-parameter

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean

install: all
	sudo insmod $(MODULE_NAME).ko

uninstall:
	-sudo rmmod $(MODULE_NAME)

reload: uninstall install

# Build and run the userspace test
test: all
	$(CC) -Wall -Werror -O2 -o test/test_basic test/test_basic.c
	@echo "=== Load module first: sudo insmod ksvc.ko ==="
	@echo "=== Then run: sudo ./test/test_basic ==="

# DKMS support
DKMS_VERSION := 0.2.0

dkms-install:
	sudo mkdir -p /usr/src/$(MODULE_NAME)-$(DKMS_VERSION)
	sudo cp *.c *.h Makefile dkms.conf /usr/src/$(MODULE_NAME)-$(DKMS_VERSION)/
	sudo dkms add -m $(MODULE_NAME) -v $(DKMS_VERSION)
	sudo dkms build -m $(MODULE_NAME) -v $(DKMS_VERSION)
	sudo dkms install -m $(MODULE_NAME) -v $(DKMS_VERSION)

dkms-remove:
	-sudo dkms remove -m $(MODULE_NAME) -v $(DKMS_VERSION) --all

.PHONY: all clean install uninstall reload test dkms-install dkms-remove
