#!/usr/bin/env python3
"""Fail closed unless nightly rustdoc reports 100% item and eligible-example coverage."""
import argparse
import json
from pathlib import Path


def check(data):
    if not isinstance(data, dict) or not data:
        raise ValueError('expected a nonempty rustdoc coverage object')
    keys = ('total', 'with_docs', 'total_examples', 'with_examples')
    totals = dict.fromkeys(keys, 0)
    failures = []
    for name, row in sorted(data.items()):
        if not isinstance(row, dict) or any(type(row.get(key)) is not int or row[key] < 0 for key in keys):
            raise ValueError(f'{name}: invalid coverage counters')
        if row['with_docs'] > row['total'] or row['with_examples'] > row['total_examples']:
            raise ValueError(f'{name}: impossible coverage counters')
        for key in keys:
            totals[key] += row[key]
        if row['total'] != row['with_docs'] or row['total_examples'] != row['with_examples']:
            failures.append(f"{name}: docs {row['with_docs']}/{row['total']}, examples {row['with_examples']}/{row['total_examples']}")
    if not totals['total'] or not totals['total_examples']:
        raise ValueError('coverage unexpectedly contains no eligible items/examples')
    if failures:
        raise ValueError('\n'.join(failures))
    return totals


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('coverage', type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(check(json.loads(args.coverage.read_text()))))
    except (OSError, ValueError, TypeError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
