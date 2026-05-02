#!/bin/sh

set -eu

cd "$(dirname "$0")/.."

cargo readme --template README.template.md > README.md
