#!/bin/zsh
# Generates synthetic benchmark audio (macOS `say` TTS + generated silence/noise)
# into test-audio/ (git-ignored). Each speech WAV gets a .txt reference transcript.
# TTS is cleaner than real speech: treat results as a lower bound on error rates
# and confirm with real microphone recordings.
set -euo pipefail

OUT="${0:A:h}/../test-audio"
mkdir -p "$OUT"
cd "$OUT"

speak() { # name voice rate text
  local name=$1 voice=$2 rate=$3 text=$4
  say -v "$voice" -r "$rate" --data-format=LEI16@48000 -o "$name.wav" "$text"
  print -r -- "$text" > "$name.txt"
}

SHORT="Please send the quarterly report to Maria by Friday afternoon."
NAMES="On March fourteenth, FCI Tradecraft reviewed twelve hundred wire transfers totaling four point seven million dollars. The FinCEN and OFAC checks flagged three accounts for KYC review under the BSA and AML rules."
LONG="Good morning everyone. I want to walk through the plan for next quarter before we open it up for questions. First, the compliance team will finish the transaction monitoring review that started in September. We found that several alert rules were tuned for a much smaller customer base, so they generate too many false positives and slow down the investigators. Second, we will update the customer due diligence procedures so that higher risk accounts get a fresh review every twelve months instead of every twenty four. Third, training. Every analyst will complete a refresher on sanctions screening, including how to document a potential match and when to escalate it to the BSA officer. Finally, I would like each manager to send me a short note by the end of next week describing the biggest obstacle their team is facing. Keep it to one page. Thank you, and let's get started."

speak short_5s      "Samantha (English (US))" 175 "$SHORT"
speak names_15s     "Samantha (English (US))" 165 "$NAMES"
speak long_60s      "Samantha (English (US))" 170 "$LONG"
speak short_uk_5s   "Daniel"                  175 "$SHORT"
speak names_ie_15s  "Moira"                   165 "$NAMES"

# Stereo 44.1 kHz float copy — exercises downmix + a different resample ratio.
afconvert -f WAVE -d LEF32@44100 -c 2 names_15s.wav names_stereo44k_15s.wav
cp names_15s.txt names_stereo44k_15s.txt

# Silence, low-level noise, and speech mixed with noise (no reference for the
# first two: the correct output is empty text).
/usr/bin/python3 -I - "$OUT" <<'PY'
import random, struct, sys, wave
out = sys.argv[1]
rate = 48000
def write(name, samples):
    with wave.open(f"{out}/{name}.wav", "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(rate)
        w.writeframes(b"".join(struct.pack("<h", max(-32768, min(32767, int(s * 32767)))) for s in samples))
random.seed(7)
write("silence_5s", [0.0] * rate * 5)
write("noise_5s", [random.gauss(0, 0.02) for _ in range(rate * 5)])
with wave.open(f"{out}/names_15s.wav", "rb") as r:
    data = r.readframes(r.getnframes())
speech = [v / 32768 for (v,) in struct.iter_unpack("<h", data)]
write("names_noisy_15s", [s + random.gauss(0, 0.03) for s in speech])
PY
cp names_15s.txt names_noisy_15s.txt

for f in *.wav; do
  printf "%-28s %s\n" "$f" "$(afinfo "$f" | awk -F': ' '/estimated duration/ {print $2}')"
done
