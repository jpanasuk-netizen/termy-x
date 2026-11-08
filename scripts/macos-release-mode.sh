#!/usr/bin/env bash
# Print "signed" or "unsigned" for the macOS release job.
# An empty Apple credential set is an unsigned build. It is not notarized.
# A partial set is an error: this script will not call that an unsigned success
# and will not call it signed.
set -euo pipefail

names=(
  APPLE_TEAM_ID
  APPLE_CERTIFICATE_P12_BASE64
  APPLE_CERTIFICATE_PASSWORD
  APPLE_NOTARY_KEY_P8_BASE64
  TERMY_NOTARY_KEY_ID
  TERMY_NOTARY_ISSUER
)

present=0
for name in "${names[@]}"; do
  if [[ -n "${!name:-}" ]]; then
    present=1
    break
  fi
done

if [[ "$present" -eq 0 ]]; then
  echo "unsigned"
  exit 0
fi

if [[ ! "${APPLE_TEAM_ID:-}" =~ ^[A-Z0-9]{10}$ ]]; then
  echo "Error: Apple signing credentials are set, but APPLE_TEAM_ID is not a 10-character team ID. Refusing to build an unsigned DMG and label it signed." >&2
  exit 1
fi

for name in "${names[@]}"; do
  if [[ -z "${!name:-}" ]]; then
    echo "Error: Missing $name. Refusing a partial Apple signing setup." >&2
    exit 1
  fi
done

echo "signed"
