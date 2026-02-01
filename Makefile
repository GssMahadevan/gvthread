# GVThread Makefile
# 
# Usage:
#   make              - Show help
#   make help         - Show help
#   make build-all    - Build all targets
#   make bench-all    - Run all benchmarks
#
# Filtering:
#   MF=bench make     - Show only targets containing 'bench'
#   MCAT=build make   - Show only Build category
#   MA=1 make         - Show flat list of all targets
#
# Environment overrides:
#   GVT_NUM_WORKERS=8 make bench-rust

.DEFAULT_GOAL := help

# Project paths
PROJECT_ROOT := $(CURDIR)
SCRIPTS_DIR  := $(PROJECT_ROOT)/scripts
MAKEFILES_DIR := $(PROJECT_ROOT)/makefiles

# Tools
PYTHON := python3
CARGO  := cargo
GO     := go

# Include subsystem makefiles
include $(MAKEFILES_DIR)/build.mk
include $(MAKEFILES_DIR)/test.mk
include $(MAKEFILES_DIR)/bench.mk
include $(MAKEFILES_DIR)/perf.mk
include $(MAKEFILES_DIR)/clean.mk

# Help target using Python parser
.PHONY: help
help: ## Show this help message
	@$(PYTHON) $(SCRIPTS_DIR)/makehelper.py $(MAKEFILE_LIST)

# Bash completion helper (used by completion script)
.PHONY: _targets
_targets:
	@MPAT=1 $(PYTHON) $(SCRIPTS_DIR)/makehelper.py $(MAKEFILE_LIST)