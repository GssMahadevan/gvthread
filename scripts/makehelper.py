#!/usr/bin/env python3
"""
makehelper.py - Parse Makefiles and generate help documentation.

Extracts target names and their ## documentation comments from Makefiles,
categorizes them, and prints formatted help output.

Usage:
  python3 makehelper.py <makefile1> [<makefile2> ...]

Environment variables:
  MF    - Filter targets by substring (e.g., MF=bench shows targets containing 'bench')
  MCAT  - Filter by category (e.g., MCAT=build shows 'Build Commands')
  MA    - Show all targets in a flat list (set to any value, e.g., MA=1)
  MPAT  - Output target patterns for bash completion (set to any value)
  DEBUG - Show debug information (set to any value)

Target documentation format in Makefiles:
  target-name: deps  ## Description of the target

Example:
  bench-go: ## Run Go benchmark
  bench-rust: ## Run Rust benchmark
"""

import sys
import re
import os
from collections import defaultdict, OrderedDict


def extract_targets(makefiles):
    """Extract targets and their documentation from a list of Makefiles."""
    targets = defaultdict(list)
    debug_mode = os.environ.get('DEBUG')
    
    # Pattern: target-name: [deps] ## description
    target_pattern = re.compile(r'^([a-zA-Z0-9_-]+):[^#]*##\s*(.*)')
    
    for makefile in makefiles:
        if not os.path.isfile(makefile):
            if debug_mode:
                print(f"Skipping non-existent file: {makefile}", file=sys.stderr)
            continue
        
        file_basename = os.path.basename(makefile)
        if debug_mode:
            print(f"Processing: {makefile}", file=sys.stderr)
            
        with open(makefile, 'r') as f:
            line_num = 0
            for line in f:
                line_num += 1
                line = line.rstrip('\n')
                
                # Skip empty lines and pure comments
                stripped = line.strip()
                if not stripped or stripped.startswith('#'):
                    continue
                
                match = target_pattern.match(line)
                if match:
                    target_name = match.group(1)
                    description = match.group(2).strip()
                    
                    if debug_mode:
                        print(f"  {line_num}: {target_name} -> {description}", file=sys.stderr)
                    
                    targets[target_name].append({
                        'name': target_name,
                        'description': description,
                        'file': file_basename
                    })
    
    return targets


def categorize_targets(targets):
    """Categorize targets based on their name prefixes."""
    categories = OrderedDict([
        ('Build', []),
        ('Test', []),
        ('Benchmark', []),
        ('Performance', []),
        ('Config', []),
        ('Docker', []),
        ('Clean', []),
        ('Help', []),
        ('Other', []),
    ])
    
    # Prefix to category mapping
    prefix_map = {
        'build': 'Build',
        'compile': 'Build',
        'test': 'Test',
        'check': 'Test',
        'bench': 'Benchmark',
        'perf': 'Performance',
        'profile': 'Performance',
        'flame': 'Performance',
        'config': 'Config',
        'cfg': 'Config',
        'docker': 'Docker',
        'container': 'Docker',
        'clean': 'Clean',
        'distclean': 'Clean',
        'help': 'Help',
    }
    
    for target_name, target_list in targets.items():
        if not target_list:
            continue
            
        target_info = target_list[0]  # Use first definition
        categorized = False
        
        # Check prefixes
        for prefix, category in prefix_map.items():
            if target_name.startswith(prefix):
                categories[category].append(target_info)
                categorized = True
                break
        
        if not categorized:
            categories['Other'].append(target_info)
    
    # Sort targets within each category
    for category in categories:
        categories[category].sort(key=lambda x: x['name'])
    
    return categories


def filter_targets(categories, filter_text='', category_filter=''):
    """Filter targets by text and/or category."""
    filtered = OrderedDict()
    
    filter_text = filter_text.lower()
    category_filter = category_filter.lower()
    
    for category, targets in categories.items():
        # Category filter
        if category_filter and category_filter not in category.lower():
            continue
        
        # Text filter
        if filter_text:
            filtered_targets = [
                t for t in targets
                if filter_text in t['name'].lower() or filter_text in t['description'].lower()
            ]
        else:
            filtered_targets = targets
        
        if filtered_targets:
            filtered[category] = filtered_targets
    
    return filtered


def print_help(categories, flat_list=False):
    """Print formatted help output."""
    if flat_list:
        # Flat sorted list of all targets
        all_targets = []
        for targets in categories.values():
            all_targets.extend(targets)
        all_targets.sort(key=lambda x: x['name'])
        
        max_name_len = max((len(t['name']) for t in all_targets), default=0)
        for target in all_targets:
            print(f"  {target['name']:<{max_name_len}}  {target['description']}")
        return
    
    # Categorized output
    for category, targets in categories.items():
        if not targets:
            continue
        
        print(f"\n\033[1;36m=== {category} Commands ===\033[0m")
        
        max_name_len = max((len(t['name']) for t in targets), default=0)
        for target in targets:
            print(f"  \033[1;32m{target['name']:<{max_name_len}}\033[0m  {target['description']}")


def print_completion_patterns(categories):
    """Output target names for bash completion."""
    all_targets = set()
    for targets in categories.values():
        for t in targets:
            all_targets.add(t['name'])
    
    # Print space-separated list
    print(' '.join(sorted(all_targets)))


def main():
    if len(sys.argv) < 2:
        print("Usage: makehelper.py MAKEFILE1 [MAKEFILE2 ...]", file=sys.stderr)
        print("\nEnvironment variables:", file=sys.stderr)
        print("  MF=<text>   Filter targets by substring", file=sys.stderr)
        print("  MCAT=<cat>  Filter by category", file=sys.stderr)
        print("  MA=1        Show all targets in flat list", file=sys.stderr)
        print("  MPAT=1      Output patterns for bash completion", file=sys.stderr)
        sys.exit(1)
    
    # Get options from environment
    filter_text = os.environ.get('MF', '')
    category_filter = os.environ.get('MCAT', '')
    show_all = os.environ.get('MA')
    show_patterns = os.environ.get('MPAT')
    
    makefiles = sys.argv[1:]
    targets = extract_targets(makefiles)
    categories = categorize_targets(targets)
    
    # Apply filters
    filtered = filter_targets(categories, filter_text, category_filter)
    
    # Output based on mode
    if show_patterns:
        print_completion_patterns(filtered)
    else:
        print_help(filtered, flat_list=bool(show_all))
        
        # Show filter note if filters applied
        if filter_text or category_filter:
            notes = []
            if filter_text:
                notes.append(f"MF='{filter_text}'")
            if category_filter:
                notes.append(f"MCAT='{category_filter}'")
            print(f"\n\033[0;33mFiltered by: {', '.join(notes)}\033[0m")
            print("Run 'make help' without filters to see all targets.")


if __name__ == "__main__":
    main()