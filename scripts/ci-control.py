#!/usr/bin/env python3
from datetime import datetime, timedelta, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile


OS = ['ubuntu-latest', 'macos-latest', 'windows-latest']
WORKFLOW = '.github/workflows/ci.yml'
SCHEMA = 1


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def git(*args):
    return command('git', *args)


def api(path):
    return json.loads(command('gh', 'api', path))


def pages(path, key=None):
    result = []
    for page in range(1, 21):
        value = api(f'{path}{"&" if "?" in path else "?"}per_page=100&page={page}')
        items = value[key] if key else value
        if not isinstance(items, list):
            raise ValueError('Invalid API collection')
        result.extend(items)
        if len(items) < 100:
            return result
    raise ValueError('API collection exceeds bounded pagination')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def fingerprint():
    paths = git('ls-files', '-z', '.github/workflows', 'scripts', '.cargo', '.config',
                'Cargo.toml', ':(glob)**/Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml').split('\0')
    digest = hashlib.sha256()
    for name in sorted(filter(None, paths)):
        digest.update(name.encode() + b'\0' + Path(name).read_bytes() + b'\0')
    return digest.hexdigest()


def live_pr(repo, number):
    pr = api(f'repos/{repo}/pulls/{number}')
    require(type(pr['draft']) is bool, 'Invalid draft state')
    return pr


def identity(pr):
    return {'number': pr['number'], 'head': pr['head']['sha'],
            'base': pr['base']['sha'], 'base_ref': pr['base']['ref'],
            'head_repo': pr['head']['repo']['full_name'],
            'base_repo': pr['base']['repo']['full_name']}


def validate_live(plan, pr):
    require(pr['state'] == 'open', 'PR is no longer open')
    require(identity(pr) == plan['pr'], 'PR base/head identity changed; rerun CI')
    require(pr['draft'] == (plan['mode'] == 'draft'), 'PR draft state changed; rerun CI')


def full_plan():
    return {'mode': 'full', 'packages': '*', 'libs': '*',
            'matrix': {'include': [{'os': name, 'label': name} for name in OS]},
            'integration': True, 'reasons': ['Full validation required']}


def skipped_plan(mode):
    return {'mode': mode, 'packages': [], 'libs': [], 'matrix': {'include': []},
            'integration': False, 'reasons': [mode]}


def validate_proof(proof, run, pr, plan, checked_commit):
    require(type(proof.get('schema')) is int and type(proof.get('run_id')) is int
            and type(proof.get('run_attempt')) is int, 'Invalid proof schema types')
    expected = {'schema': SCHEMA, 'mode': 'full', 'repository': plan['repository'],
                'run_id': run['id'], 'run_attempt': run['run_attempt'],
                'tree': plan['tree'], 'config': plan['config'],
                'rustc': plan['rustc'], 'workflow': WORKFLOW}
    require(all(proof.get(k) == v for k, v in expected.items()), 'Proof identity mismatch')
    require(proof['pr']['number'] == pr['number'] and proof['pr']['head'] == pr['head']['sha'],
            'Proof PR mismatch')
    require(proof['pr']['base_ref'] == 'main'
            and proof['pr']['head_repo'] == plan['repository']
            and proof['pr']['base_repo'] == plan['repository'], 'Proof must be same-repository main PR')
    require(checked_commit['tree']['sha'] == proof['tree'], 'Checked merge tree mismatch')
    require([p['sha'] for p in checked_commit['parents']] == [proof['pr']['base'], proof['pr']['head']],
            'Checked merge parents mismatch')
    require(proof['checks'] == [f'check ({name})' for name in OS] + ['integration', 'progress'],
            'Proof is not complete full validation')
    require(run['event'] == 'pull_request' and run['status'] == 'completed'
            and run['conclusion'] == 'success' and run['path'] == WORKFLOW
            and run['head_sha'] == proof['pr']['head']
            and run['repository']['full_name'] == plan['repository']
            and run['head_repository']['full_name'] == plan['repository'], 'Invalid source run')
    # GitHub clears this association after merging. The commit-associated merged
    # PR, immutable proof, run head/repositories, and merge parents bind that case.
    require(isinstance(run['pull_requests'], list), 'Invalid run PR associations')
    require(not run['pull_requests'] or any(
        p['number'] == pr['number'] and p['head']['sha'] == proof['pr']['head']
        and p['base']['ref'] == 'main' for p in run['pull_requests']), 'Run is not bound to PR')


def read_artifact(repo, artifact):
    require(not artifact['expired'] and artifact['size_in_bytes'] < 65536, 'Invalid proof artifact')
    raw = subprocess.check_output(['gh', 'api', f'repos/{repo}/actions/artifacts/{artifact["id"]}/zip'])
    require(len(raw) < 65536, 'Oversized artifact download')
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        require(archive.namelist() == ['proof.json'], 'Unexpected artifact contents')
        require(archive.getinfo('proof.json').file_size < 16384, 'Oversized proof')
        return json.loads(archive.read('proof.json'))


def warm_cache_seed(plan):
    # This proves a recent eligible seed, not that GitHub has retained caches.
    repo = plan['repository']
    since = datetime.now(timezone.utc) - timedelta(days=6)
    cutoff = since.strftime('%Y-%m-%dT%H:%M:%SZ')
    runs = pages(f'repos/{repo}/actions/workflows/ci.yml/runs?branch=main&status=success&created=%3E%3D{cutoff}', 'workflow_runs')
    for run in runs:
        if (run['event'] not in ('push', 'workflow_dispatch') or run['head_branch'] != 'main'
                or run['conclusion'] != 'success' or run['status'] != 'completed'
                or run['path'] != WORKFLOW or run['repository']['full_name'] != repo
                or datetime.fromisoformat(run['created_at'].replace('Z', '+00:00')) < since):
            continue
        artifacts = pages(f'repos/{repo}/actions/runs/{run["id"]}/artifacts', 'artifacts')
        name = f'ci-main-cache-state-{run["id"]}-{run["run_attempt"]}'
        matching = [a for a in artifacts if a['name'] == name]
        if len(matching) != 1:
            continue
        proof = read_artifact(repo, matching[0])
        expected = {'schema': SCHEMA, 'mode': 'full', 'repository': repo,
                    'run_id': run['id'], 'run_attempt': run['run_attempt'],
                    'commit': run['head_sha'], 'config': plan['config'],
                    'rustc': plan['rustc'], 'workflow': WORKFLOW,
                    'checks': [f'check ({name})' for name in OS] + ['integration', 'progress']}
        if all(proof.get(key) == value for key, value in expected.items()):
            return
    raise ValueError('No recent matching full-main cache seed; refresh with full checks')


def reusable(plan):
    # Commit association is the only entry point. Missing/racing APIs fail closed.
    warm_cache_seed(plan)
    repo, sha = plan['repository'], plan['commit']
    pulls = pages(f'repos/{repo}/commits/{sha}/pulls')
    matches = [p for p in pulls if p.get('merged_at') and p['merge_commit_sha'] == sha
               and p['base']['ref'] == 'main' and p['head']['repo']
               and p['head']['repo']['full_name'] == repo and p['base']['repo']['full_name'] == repo]
    require(len(matches) == 1, 'No unique same-repository merged PR for this commit')
    pr = live_pr(repo, matches[0]['number'])
    require(pr['merged'] and pr['merge_commit_sha'] == sha and pr['base']['ref'] == 'main'
            and pr['head']['repo']['full_name'] == repo and pr['base']['repo']['full_name'] == repo,
            'Merged PR identity changed')
    runs = pages(f'repos/{repo}/actions/workflows/ci.yml/runs?event=pull_request&head_sha={pr["head"]["sha"]}&status=success', 'workflow_runs')
    for candidate in runs:
        try:
            run = api(f'repos/{repo}/actions/runs/{candidate["id"]}')
            artifacts = pages(f'repos/{repo}/actions/runs/{run["id"]}/artifacts', 'artifacts')
            name = f'ci-full-proof-{run["id"]}-{run["run_attempt"]}'
            matching = [a for a in artifacts if a['name'] == name]
            require(len(matching) == 1, 'Missing unique immutable proof artifact')
            proof = read_artifact(repo, matching[0])
            checked_commit = api(f'repos/{repo}/git/commits/{proof["commit"]}')
            validate_proof(proof, run, pr, plan, checked_commit)
            return {'run_id': run['id'], 'run_attempt': run['run_attempt'], 'artifact_id': matching[0]['id']}
        except (KeyError, ValueError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
            print(f'Cannot reuse candidate: {error}', file=sys.stderr)
    raise ValueError('No completed successful exact-tree full proof')


def scope():
    event = os.environ['EVENT']
    repo = os.environ['GITHUB_REPOSITORY']
    common = {'repository': repo, 'commit': git('rev-parse', 'HEAD'),
              'tree': git('rev-parse', 'HEAD^{tree}'), 'config': fingerprint(),
              'rustc': command('rustc', '--version'), 'event': event}
    if event == 'pull_request':
        pr = live_pr(repo, int(os.environ['PULL_REQUEST']))
        require(pr['state'] == 'open', 'PR is not open')
        require(git('rev-parse', 'HEAD^1') == pr['base']['sha']
                and git('rev-parse', 'HEAD^2') == pr['head']['sha'], 'Checkout differs from live PR base/head')
        if pr['draft']:
            plan = skipped_plan('draft')
        else:
            plan = json.loads(command('python3', 'scripts/ci-plan.py', '--base', pr['base']['sha'],
                                      '--base-ref', pr['base']['ref']))
        common['pr'] = identity(pr)
    else:
        plan = full_plan()
        if event == 'push' and os.environ.get('GITHUB_REF') == 'refs/heads/main':
            try:
                common['reuse'] = reusable(common)
                plan = skipped_plan('reuse')
            except Exception as error:
                # Optimization failure must never suppress verification.
                print(f'Full validation fallback: {error}', file=sys.stderr)
    plan.update(common)
    Path('.ci-plan.json').write_text(json.dumps(plan))
    outputs = {'plan': json.dumps(plan, separators=(',', ':')), 'mode': plan['mode'],
               'matrix': json.dumps(plan['matrix'], separators=(',', ':')),
               'integration': str(plan['integration']).lower(),
               'run_checks': str(bool(plan['matrix']['include'])).lower()}
    with open(os.environ.get('GITHUB_OUTPUT', '/dev/stdout'), 'a') as output:
        for key, value in outputs.items():
            print(f'{key}={value}', file=output)


def validate_results(plan, needs, jobs):
    modes = {'pull_request': ('full', 'fast', 'docs', 'draft'),
             'push': ('full', 'reuse'), 'workflow_dispatch': ('full',)}
    require(plan['mode'] in modes[plan['event']], 'Invalid mode for event')
    require(type(plan['integration']) is bool, 'Invalid integration flag')
    if plan['event'] == 'pull_request' and plan['pr']['base_ref'] == 'main':
        require(plan['mode'] in ('full', 'draft'), 'Main PR requires full validation')
    selected = [item['os'] for item in plan['matrix']['include']]
    require(len(selected) == len(set(selected)) and all(name in OS for name in selected),
            'Invalid platform selection')
    if plan['mode'] in ('full', 'fast'):
        require(bool(selected), 'Code checks require at least one platform')
    else:
        require(not selected and plan['integration'] is False, 'Skipped plan has unexpected checks')
    require(needs['scope']['result'] == 'success', 'Scope did not succeed')
    require(needs['progress']['result'] == 'success', 'Repository checks did not succeed')
    checks = bool(plan['matrix']['include'])
    require(needs['check']['result'] == ('success' if checks else 'skipped'), 'Unexpected check result')
    require(needs['integration']['result'] == ('success' if plan['integration'] else 'skipped'),
            'Unexpected database result')
    require(needs['binaries']['result'] == ('skipped' if plan['event'] == 'pull_request' else 'success'),
            'Unexpected CLI build/smoke result')
    expected = [f'check ({item["os"]})' for item in plan['matrix']['include']] + ['progress']
    if plan['integration']:
        expected.append('integration')
    if plan['event'] != 'pull_request':
        expected.extend(f'binaries ({name})' for name in OS)
    if plan['mode'] == 'full':
        require(plan['packages'] == '*' and plan['libs'] == '*'
                and [i['os'] for i in plan['matrix']['include']] == OS and plan['integration'],
                'Full plan must include all platforms and databases')
    for name in expected:
        matching = [j for j in jobs if j['name'] == name]
        require(len(matching) == 1 and matching[0]['status'] == 'completed'
                and matching[0]['conclusion'] == 'success', f'Missing successful job: {name}')


def latest_jobs(jobs, run_id, head, attempt):
    selected = {}
    for job in jobs:
        require(job['run_id'] == run_id and job['head_sha'] == head
                and 1 <= job['run_attempt'] <= attempt, 'Job belongs to different source/run attempt')
        previous = selected.get(job['name'])
        if previous is None or previous['run_attempt'] < job['run_attempt']:
            selected[job['name']] = job
        elif previous['run_attempt'] == job['run_attempt']:
            require(previous['id'] == job['id'], 'Duplicate job identity within attempt')
    return list(selected.values())


def gate():
    plan = json.loads(os.environ['PLAN'])
    require(plan['commit'] == git('rev-parse', 'HEAD') and plan['tree'] == git('rev-parse', 'HEAD^{tree}')
            and plan['config'] == fingerprint(), 'Gate checkout differs from planned source')
    repo = os.environ['GITHUB_REPOSITORY']
    require(plan['repository'] == repo and plan['event'] == os.environ['GITHUB_EVENT_NAME'],
            'Plan event/repository mismatch')
    run_id, attempt = int(os.environ['GITHUB_RUN_ID']), int(os.environ['GITHUB_RUN_ATTEMPT'])
    jobs = pages(f'repos/{repo}/actions/runs/{run_id}/jobs?filter=all', 'jobs')
    head = plan['pr']['head'] if plan['event'] == 'pull_request' else plan['commit']
    jobs = latest_jobs(jobs, run_id, head, attempt)
    validate_results(plan, json.loads(os.environ['NEEDS']), jobs)
    if plan['event'] == 'pull_request':
        validate_live(plan, live_pr(repo, plan['pr']['number']))
    if plan['mode'] == 'full':
        # Each receipt comes from an OS job after all of its steps succeeded.
        for name in OS + ['integration']:
            receipt = json.loads(Path(f'.ci-receipts/{name}/receipt.json').read_text())
            require(receipt == {'os': name, 'rustc': plan['rustc'], 'commit': plan['commit']},
                    'OS receipt/toolchain mismatch')
        proof = {key: plan[key] for key in ('repository', 'commit', 'tree', 'config', 'rustc')}
        if plan['event'] == 'pull_request':
            proof['pr'] = plan['pr']
        proof.update(schema=SCHEMA, mode='full', workflow=WORKFLOW, run_id=run_id,
                     run_attempt=attempt, checks=[f'check ({name})' for name in OS] + ['integration', 'progress'])
        Path('proof.json').write_text(json.dumps(proof, sort_keys=True))
    print(f'CI required: {plan["mode"]} passed')


if __name__ == '__main__':
    try:
        {'scope': scope, 'gate': gate}[sys.argv[1]]()
    except Exception as error:
        print(f'::error::{error}', file=sys.stderr)
        sys.exit(1)
