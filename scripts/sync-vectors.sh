#!/bin/sh
# Copy the conformance vectors out of a kenari-pay checkout, byte for byte.
#
#   scripts/sync-vectors.sh <path to the kenari-pay checkout>
#
# vectors/ ends up holding exactly contract/*.json and contracts/vectors/addresses.json of that
# checkout, so a file the gateway removed disappears here too. The gateway repo has a test
# (crates/core/tests/sdk_vectors.rs) that compares the two directories when this checkout sits
# next to it, and the tests of this crate read nothing else.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/sync-vectors.sh <kenari-pay checkout>" >&2
    exit 1
fi

src=$1
here=$(cd "$(dirname "$0")/.." && pwd)

if [ ! -d "$src/contract" ] || [ ! -f "$src/contracts/vectors/addresses.json" ]; then
    echo "not a kenari-pay checkout: $src has no contract/ or contracts/vectors/addresses.json" >&2
    exit 1
fi

mkdir -p "$here/vectors"
rm -f "$here"/vectors/*.json
cp "$src"/contract/*.json "$here/vectors/"
cp "$src/contracts/vectors/addresses.json" "$here/vectors/addresses.json"

echo "synced $(ls "$here"/vectors/*.json | wc -l | tr -d ' ') files into vectors/"
