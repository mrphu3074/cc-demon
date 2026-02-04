#!/bin/bash
# Session start hook: Check demon status and show recent job completions
# This runs silently on session start to provide context

DEMON_DIR="${HOME}/.demon"
OUTPUT_DIR="${DEMON_DIR}/output"

# Check if demon binary exists
if ! command -v demon &> /dev/null; then
    exit 0
fi

# Check if daemon is running
if demon status 2>/dev/null | grep -q "running"; then
    # Check for recent job outputs (last hour)
    if [ -d "$OUTPUT_DIR" ]; then
        recent=$(find "$OUTPUT_DIR" -name "*.md" -mmin -60 2>/dev/null | wc -l)
        if [ "$recent" -gt 0 ]; then
            echo "CC-Demon: ${recent} job(s) completed in the last hour. Run /demon:status to see details."
        fi
    fi
fi

exit 0
