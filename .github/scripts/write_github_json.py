#!/usr/bin/env python3
"""Write GITHUB_JSON from the environment to a file. Used by pr_targets workflow."""
import os
import sys


def main() -> None:
    json_str = os.environ.get("GITHUB_JSON", "")
    if not json_str:
        sys.stderr.write("GITHUB_JSON environment variable is not set\n")
        sys.exit(1)
    if len(sys.argv) < 2:
        sys.stderr.write("Usage: write_github_json.py <output_path>\n")
        sys.exit(1)
    path = sys.argv[1]
    with open(path, "w") as f:
        f.write(json_str)


if __name__ == "__main__":
    main()
