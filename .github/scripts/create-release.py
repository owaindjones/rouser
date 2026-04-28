#!/usr/bin/env python3
"""Create or update a GitHub Release via REST API."""
import sys, json, os, urllib.request


def main():
    token = os.environ["GITHUB_TOKEN"]
    repo = os.environ.get("GH_REPO", "owaindjones/rouser")
    
    # Extract version from GITHUB_REF (e.g., refs/tags/v1.2.3 -> 1.2.3)
    github_ref = os.environ.get("GITHUB_REF", "")
    if not github_ref.startswith("refs/tags/v"):
        print(f"ERROR: Unexpected GITHUB_REF format: {github_ref}", file=sys.stderr)
        sys.exit(1)
    version = github_ref[len("refs/tags/v"):]

    base_url = f"https://api.github.com/repos/{repo}/releases"
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github.v3+json",
    }

    # Check if release already exists by tag
    try:
        req = urllib.request.Request(
            f"{base_url}/tags/v{version}", headers=headers
        )
        resp = urllib.request.urlopen(req, timeout=30)
        existing_id = json.load(resp)["id"]
        print(f"Release v{version} already exists (ID: {existing_id}), updating...")
        method = "PATCH"
        url = f"{base_url}/{existing_id}"
    except Exception as e:
        err_msg = str(e) if hasattr(e, "read") else str(e)
        print(f"No existing release found ({err_msg}), creating new one...")
        method = "POST"
        url = base_url

    # Read notes file
    with open("RELEASE_NOTES.md", "r") as f:
        body = f.read()

    payload = json.dumps({
        "tag_name": f"v{version}",
        "name": f"v{version}",
        "body": body,
    }).encode()

    req = urllib.request.Request(
        url, data=payload, headers={**headers, "Content-Type": "application/json"}, method=method
    )
    try:
        resp = urllib.request.urlopen(req, timeout=30)
        result = json.load(resp)
        html_url = result.get("html_url", "unknown")
        print(f"Release v{version}: {html_url}")
    except Exception as e:
        print(f"ERROR: Release creation/update failed: {e}", file=sys.stderr)
        if hasattr(e, "read"):
            err_body = e.read().decode() if hasattr(e, "read") else str(e)
            print(f"API response: {err_body[:500]}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
