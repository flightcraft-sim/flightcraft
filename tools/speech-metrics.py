#!/usr/bin/env python3
"""Add digit-canonical WER alongside raw WER in speech measurement receipts.

Only literal digits become spoken digit words, and 'niner' becomes 'nine'. No
facilities, phrases, callsigns, or wrong numeric values are repaired. Thus 06
and 'zero six' compare equally, while 24 and 06 remain different.
"""
import json
from pathlib import Path
import re
import sys

DIGITS = ('zero', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine')


def tokens(text):
    result = []
    for word in re.findall(r'[a-z0-9]+', text.lower()):
        if word.isascii() and word.isdigit():
            result.extend(DIGITS[int(c)] for c in word)
        else:
            result.append('nine' if word == 'niner' else word)
    return result


def errors(reference, recognized):
    left, right = tokens(reference), tokens(recognized)
    previous = list(range(len(right) + 1))
    for i, word in enumerate(left):
        current = [i + 1]
        for j, other in enumerate(right):
            current.append(min(previous[j] + (word != other), previous[j + 1] + 1, current[j] + 1))
        previous = current
    return previous[-1], len(left)


assert errors('runway zero six Sling three seven one Foxtrot Charlie', 'Runway 06 Sling 3-7-1 Foxtrot Charlie') == (0, 9)
assert errors('runway zero six', 'runway 24')[0] == 2
def main():
    for argument in sys.argv[1:]:
        path = Path(argument)
        report = json.loads(path.read_text())
        groups = report.get('models', [report])
        for group in groups:
            wrong = count = 0
            for row in group['calls']:
                if '-retry' in row['file']:
                    continue
                error, words = errors(row['reference'], row['recognized'])
                row['digit_canonical_word_errors'] = error
                row['digit_canonical_reference_words'] = words
                wrong += error
                count += words
            group['digit_canonical_word_errors'] = wrong
            group['digit_canonical_reference_words'] = count
            group['digit_canonical_wer'] = wrong / count if count else None
        report['digit_canonical_wer_method'] = __doc__.strip()
        path.write_text(json.dumps(report, indent=2) + '\n')
        print(path)


if __name__ == "__main__":
    main()
