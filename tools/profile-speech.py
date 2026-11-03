#!/usr/bin/env python3
"""Profile the actual desktop release executable on exported live-state WAV cases.

First export cases with FLIGHTCRAFT_SPEECH_EXPORT=<directory> on the default
recorded_wavs_through_recognizer_normalizer_review_and_mcp_kabe_pattern test.
Reference text is used only here, after the executable has returned recognition.
"""
import argparse
import json
import os
from pathlib import Path
import re
import runpy
import statistics
import subprocess
import tempfile


def distance(left, right):
    previous = list(range(len(right) + 1))
    for i, word in enumerate(left):
        current = [i + 1]
        for j, other in enumerate(right):
            current.append(min(previous[j] + (word != other), previous[j+1] + 1, current[j] + 1))
        previous = current
    return previous[-1], len(left)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--context-seconds', type=int, default=10)
    parser.add_argument('--prompt-mode', choices=('state', 'legacy', 'none'), default='state')
    parser.add_argument('--repetitions', type=int, default=1)
    parser.add_argument('--models', nargs='+', default=['tiny.en', 'base.en'])
    parser.add_argument('--threads', type=int)
    args = parser.parse_args()
    cases = [json.loads(p.read_text()) for p in sorted(args.cases.glob('case-*.json'))]
    if not cases:
        parser.error('No exported cases found')
    request = dict(models=args.models, repetitions=args.repetitions,
                   audio_context_seconds=args.context_seconds, prompt_mode=args.prompt_mode,
                   cases=[{k: v for k, v in c.items() if k in ('wav', 'snapshot', 'context', 'regions')} for c in cases])
    env = os.environ.copy()
    if args.threads is None:
        env.pop('RAYON_NUM_THREADS', None)
    else:
        env['RAYON_NUM_THREADS'] = str(args.threads)
    with tempfile.NamedTemporaryFile(mode='w', suffix='.json') as f:
        json.dump(request, f)
        f.flush()
        result = subprocess.run([str(args.executable.resolve()), '--speech-benchmark', f.name],
                                env=env, text=True, stdout=subprocess.PIPE, check=True)
    report = json.loads(result.stdout)
    canonical = runpy.run_path(str(Path(__file__).with_name('speech-metrics.py')))['errors']
    cpu_info = Path('/proc/cpuinfo')
    report['cpu'] = next((s.split(':', 1)[1].strip() for s in cpu_info.read_text().splitlines()
                          if s.startswith('model name')), 'unknown') if cpu_info.exists() else 'unknown'
    report['prompt_mode'] = args.prompt_mode
    report['minimum_audio_context_seconds'] = args.context_seconds
    for model in report['models']:
        total = wrong = canon_wrong = canon_total = correct = 0
        for call in model['calls']:
            case = next(c for c in cases if c['wav'] == call['wav'])
            text = call['draft']['recognized']
            tokenize = lambda s: re.findall(r'[a-z0-9]+', s.lower())
            a, b = distance(tokenize(case['reference']), tokenize(text))
            c, d = canonical(case['reference'], text)
            wrong += a; total += b; canon_wrong += c; canon_total += d
            expected = dict(case['expected'])
            actual = dict(call['draft']['message'])
            expected.pop('normalized', None); actual.pop('normalized', None)
            phrases = ('cleared for takeoff', 'left closed traffic', 'enter left downwind', 'cleared to land', 'left crosswind', 'left downwind', 'left base', 'final', 'clear of runway', 'request closed traffic')
            matching_phrases = all((p in case['expected_normalized']) == (p in call['draft']['normalized']) for p in phrases)
            typed_correct = not call['draft']['retry'] and expected == actual and matching_phrases
            correct += int(typed_correct)
            call.update(reference=case['reference'], file=Path(call['wav']).name,
                        recognized=text, word_errors=a, reference_words=b,
                        digit_canonical_word_errors=c, digit_canonical_reference_words=d,
                        typed_correct=typed_correct)
        model.update(typed_correct=correct, total_calls=len(model['calls']), word_errors=wrong,
                     reference_words=total, wer=wrong/total, digit_canonical_wer=canon_wrong/canon_total,
                     median_release_to_typed_ms=statistics.median(c['release_to_typed_ms'] for c in model['calls']))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    for model in report['models']:
        print(model['model'], f"{model['typed_correct']}/{model['total_calls']} typed correctly; "
              f"WER {model['wer']:.2%}; digit-canonical {model['digit_canonical_wer']:.2%}; "
              f"median {model['median_release_to_typed_ms']:.1f} ms")


if __name__ == '__main__':
    main()
