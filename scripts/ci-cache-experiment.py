#!/usr/bin/env python3
# Temporary controlled comparison; not part of the production CI gate.
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET


def output(args):
    return subprocess.check_output(args, text=True).strip()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def fingerprints(target):
    result = {}
    for path in sorted((target / 'debug' / '.fingerprint').glob('*/*.json')):
        data = path.read_bytes()
        result[str(path.relative_to(target))] = {
            'sha256': hashlib.sha256(data).hexdigest(),
            'features': json.loads(data).get('features'),
        }
    return result


def clone(source, destination):
    if destination.exists() or destination.is_symlink() or source.is_symlink():
        raise RuntimeError('Snapshot requires new destination and real source directory')
    start = time.monotonic()
    # APFS clone preserves an immutable restored cache without doubling disk use.
    subprocess.run(['cp', '-cRp', str(source), str(destination)], check=True)
    return round(time.monotonic() - start, 3)


def timed(args, path, capture=False):
    print('Running:', ' '.join(args), flush=True)
    start = time.monotonic()
    with path.open('w') as log:
        if capture:
            completed = subprocess.run(args, stdout=subprocess.PIPE, stderr=log, text=True)
        else:
            completed = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, text=True)
    seconds = round(time.monotonic() - start, 3)
    text = path.read_text()
    result = {
        'command': args, 'seconds': seconds, 'returncode': completed.returncode,
        'compiling_lines': len(re.findall(r'^\s*Compiling ', text, re.M)),
        'download_lines': len(re.findall(r'^\s*Download', text, re.M)),
    }
    print(json.dumps(result), flush=True)
    if completed.returncode:
        print(text[-16000:], flush=True)
    return result, completed.stdout if capture else None


def test_inventory(data):
    tests = []
    for binary, suite in data['rust-suites'].items():
        for name, info in suite.get('testcases', {}).items():
            if info['filter-match']['status'] == 'matches':
                tests.append((suite['package-name'], binary, name, info['ignored']))
    return sorted(tests)


def junit_inventory(path):
    return sorted((case.attrib.get('classname'), case.attrib['name'])
                  for case in ET.parse(path).iter('testcase')
                  if case.find('skipped') is None)


def main():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or platform.system() != 'Darwin':
        raise RuntimeError('This prototype only operates in an ephemeral macOS Actions runner')
    root = Path.cwd().resolve()
    if root != Path(os.environ['GITHUB_WORKSPACE']).resolve():
        raise RuntimeError('Unexpected checkout directory')
    source, base = (os.environ['EXPERIMENT_' + key] for key in ('SOURCE', 'BASE'))
    if not all(re.fullmatch('[0-9a-f]{40}', value) for value in (source, base)):
        raise ValueError('Source and base must be immutable full commit IDs')
    if output(['git', 'rev-parse', 'HEAD']) != source or output(['git', 'status', '--porcelain']):
        raise RuntimeError('Measured checkout differs from the declared clean source')
    subprocess.run(['git', 'merge-base', '--is-ancestor', base, source], check=True)
    order = os.environ['EXPERIMENT_ORDER']
    if order not in ('AB', 'BA'):
        raise ValueError('Unknown experiment order')
    report = Path(os.environ['RUNNER_TEMP']) / 'scope-experiment'
    report.mkdir(exist_ok=False)
    plan = json.loads(output(['python3', 'scripts/ci-plan.py', '--base', base,
                              '--base-ref', 'integration/experiment']))
    if plan['mode'] != 'fast' or not plan['packages']:
        raise RuntimeError('Experiment requires a nonempty affected-package plan')
    if not all(re.fullmatch('[A-Za-z0-9_-]+', name) for name in plan['packages']):
        raise ValueError('Invalid package name')
    plan_path = report / 'plan.json'
    write_json(plan_path, plan)
    filterset = ' or '.join('package(=' + name + ')' for name in plan['packages'])
    selected = [part for name in plan['packages'] for part in ('-p', name)]
    metadata = {
        'source': source, 'base': base, 'prototype': os.environ['GITHUB_SHA'],
        'tree': output(['git', 'rev-parse', 'HEAD^{tree}']), 'order': order,
        'rustc': output(['rustc', '-vV']), 'nextest': output(['cargo', 'nextest', '--version']),
        'platform': platform.platform(), 'cpus': os.cpu_count(),
        'cache_hit': os.environ.get('EXPERIMENT_CACHE_HIT'),
        'cargo_env': {key: value for key, value in os.environ.items() if key.startswith('CARGO_')},
    }
    write_json(report / 'environment.json', metadata)
    # Remove network ordering bias; this does not compile or change target/.
    fetch, _ = timed(['cargo', 'fetch', '--locked'], report / 'fetch.log')
    write_json(report / 'setup.json', fetch)
    if fetch['returncode']:
        raise RuntimeError('Dependency fetch failed')
    target = root / 'target'
    if target.is_symlink():
        raise RuntimeError('Refusing a symlink target directory')
    target.mkdir(exist_ok=True)
    baseline = Path(os.environ['RUNNER_TEMP']) / 'scope-experiment-target-baseline'
    initial = fingerprints(target)
    if not initial:
        raise RuntimeError('No restored Cargo fingerprints: cannot measure warm-cache behavior')
    write_json(report / 'baseline-fingerprints.json', initial)
    snapshot_seconds = clone(target, baseline)
    results = {'snapshot_seconds': snapshot_seconds, 'modes': {}}
    inventories, executed = {}, {}
    for mode in order:
        # Only this disposable checkout's target is removed; baseline is never mutated.
        shutil.rmtree(target)
        reset_seconds = clone(baseline, target)
        if fingerprints(target) != initial:
            raise RuntimeError('Restored target differs between modes')
        destination = report / mode
        destination.mkdir()
        check = ['bash', 'scripts/check.sh', '--plan', str(plan_path)]
        commands = {
            'clippy': check + ['clippy'] if mode == 'A' else
                      ['cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'],
            'build': check + ['build'] if mode == 'A' else
                     ['cargo', 'nextest', 'run', '--workspace', '--profile', 'ci', '--no-run', '--timings'],
            'test': check + ['test'] if mode == 'A' else
                    ['cargo', 'nextest', 'run', '--workspace', '--profile', 'ci', '-E', filterset],
            'throughput': check + ['throughput'],
            'doc': check + ['doc'],
        }
        result = {'reset_seconds': reset_seconds, 'steps': {}}
        results['modes'][mode] = result
        for name, command in commands.items():
            step, _ = timed(command, destination / (name + '.log'))
            result['steps'][name] = step
            write_json(report / 'results.json', results)
            if name in ('build', 'test'):
                write_json(destination / (name + '-fingerprints.json'), fingerprints(target))
            if step['returncode']:
                raise RuntimeError(mode + ': ' + name + ' failed; comparison is invalid')
        result['seconds'] = round(sum(step['seconds'] for step in result['steps'].values()), 3)
        junit = target / 'nextest' / 'ci' / 'junit.xml'
        shutil.copy2(junit, destination / 'junit.xml')
        executed[mode] = junit_inventory(junit)
        timing = target / 'cargo-timings' / 'cargo-timing.html'
        if timing.exists():
            shutil.copy2(timing, destination / timing.name)
        # List after all timed steps: inventory compilation cannot warm measured commands.
        listing_command = ['cargo', 'nextest', 'list'] + (selected if mode == 'A' else ['--workspace'])
        if mode == 'B':
            listing_command += ['-E', filterset]
        listing_command += ['--profile', 'ci', '--message-format', 'json']
        listing, data = timed(listing_command, destination / 'list.log', capture=True)
        result['inventory'] = listing
        if listing['returncode']:
            raise RuntimeError('Cannot verify identical tests')
        (destination / 'list.json').write_text(data)
        inventories[mode] = test_inventory(json.loads(data))
        if not inventories[mode] or not executed[mode]:
            raise RuntimeError('Empty test inventory')
        write_json(destination / 'selected-tests.json', inventories[mode])
        write_json(destination / 'executed-tests.json', executed[mode])
        write_json(report / 'results.json', results)
    results['identical_selected_tests'] = inventories['A'] == inventories['B']
    results['identical_executed_tests'] = executed['A'] == executed['B']
    results['selected_tests'] = len(inventories['A'])
    results['executed_tests'] = len(executed['A'])
    results['b_vs_a_percent'] = round((results['modes']['B']['seconds'] /
                                     results['modes']['A']['seconds'] - 1) * 100, 2)
    write_json(report / 'results.json', results)
    with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as summary:
        summary.write('```json\n' + json.dumps(results, indent=2) + '\n```\n')
    if not results['identical_selected_tests'] or not results['identical_executed_tests']:
        raise RuntimeError('Test sets differ; comparison is invalid')


if __name__ == '__main__':
    main()
