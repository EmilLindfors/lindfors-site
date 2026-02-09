#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
ENV_FILE="$ROOT_DIR/.env"

if [ ! -f "$ENV_FILE" ]; then
  echo "Error: .env file not found at $ENV_FILE"
  exit 1
fi

ADMIN_KEY=$(grep -E '^ADMIN_KEY=' "$ENV_FILE" | cut -d'=' -f2-)

if [ -z "$ADMIN_KEY" ]; then
  echo "Error: ADMIN_KEY not set in .env"
  exit 1
fi

SLUG="${1:-}"
if [ -z "$SLUG" ]; then
  echo "Usage: $0 <slug> [subject]"
  echo "Example: $0 aquaculture-innovation"
  exit 1
fi

SUBJECT="${2:-}"
if [ -n "$SUBJECT" ]; then
  BODY=$(printf '{"slug":"%s","subject":"%s"}' "$SLUG" "$SUBJECT")
else
  BODY=$(printf '{"slug":"%s"}' "$SLUG")
fi

echo "Newsletter: $SLUG"
[ -n "$SUBJECT" ] && echo "Subject override: $SUBJECT"
echo ""
read -rp "Send to all subscribers? [y/N] " confirm
if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then
  echo "Aborted."
  exit 0
fi

echo "Sending..."
curl -s -X POST "https://lindfors.no/api/send-newsletter?key=$ADMIN_KEY" \
  -H 'Content-Type: application/json' \
  -d "$BODY" | python3 -m json.tool
