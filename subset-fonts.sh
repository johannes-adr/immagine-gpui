#!/bin/sh
# Shrink the Arial fonts to Latin (the build embeds assets/*.sub.ttf). Needs fonttools (pyftsubset).
set -e
U="U+0020-007E,U+00A0-00FF,U+2013,U+2014,U+2018-201E,U+2022,U+2026,U+20AC"
for f in Arial Arial-Bold; do
  pyftsubset "assets/$f.ttf" --unicodes="$U" --layout-features='kern,liga' --output-file="assets/$f.sub.ttf"
done
