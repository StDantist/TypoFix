#!/usr/bin/env bash
# Завантаження сирих корпусів для тренування LM/словників TypoFix.
#
# Джерело: Leipzig Corpora Collection (https://wortschatz-leipzig.de/) — чистий
# публічний текст (Wikipedia), по реченню на рядок, формат "<id>\t<речення>".
# Публічний → жодних особистих даних (приватність, CLAUDE.md §4).
#
# Завантажує у data/corpora/ (gitignored). Далі:
#   python data/clean_corpus.py                      # очищення -> *.clean.txt, *.words.txt
#   cargo run -p typofix-data --bin train_models     # -> data/lm/*.bin, data/dicts/*.fst
#   cargo run -p typofix-data --bin calibrate        # перепрогін метрик
#
# Мережа нестабільна → curl з ретраями. Як 100K не качається — спробуйте менший
# зріз (30K/10K): просто заміни суфікс у назві.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)/corpora"
mkdir -p "$DIR"

UK_URL="https://downloads.wortschatz-leipzig.de/corpora/ukr_wikipedia_2021_100K.tar.gz"
EN_URL="https://downloads.wortschatz-leipzig.de/corpora/eng_wikipedia_2016_100K.tar.gz"

fetch() {
  local url="$1" out="$2"
  echo "↓ $url"
  curl -sS -L --connect-timeout 20 --max-time 600 --retry 3 --retry-delay 5 -o "$out" "$url"
}

fetch "$UK_URL" "$DIR/uk.tar.gz"
fetch "$EN_URL" "$DIR/en.tar.gz"
tar -xzf "$DIR/uk.tar.gz" -C "$DIR"
tar -xzf "$DIR/en.tar.gz" -C "$DIR"
echo "Готово. Далі: python data/clean_corpus.py"
