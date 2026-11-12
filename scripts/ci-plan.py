#!/usr/bin/env python3
# Conservative CI scope shared by local checks and pull-request planning.
import argparse
import json
import subprocess
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent
OS = ['ubuntu-latest', 'macos-latest', 'windows-latest']
# Common runtime/platform boundaries require validation on every native OS.
COMMON = {'ui', 'editor', 'workbench', 'process', 'terminal', 'terminal-ui',
          'native-path', 'file-watcher', 'project', 'settings', 'app-info', 'py-env'}
DB = {'db-core', 'db-service', 'db-testkit', 'db-postgres', 'db-sqlite',
      'db-oracle-thin', 'db-oracle-odpi', 'secrets', 'settings-import'}


def command(args, root=ROOT):
    return subprocess.run(args, cwd=root, check=True, capture_output=True, text=True).stdout


def changed_files(base, root=ROOT):
    # Compare with the exact target tree, not a stale fork point. --no-renames
    # retains both paths; NUL delimiters preserve spaces and newlines.
    files = command(['git', 'diff', '--name-only', '--no-renames', '-z', base, '--'], root)
    untracked = command(['git', 'ls-files', '--others', '--exclude-standard', '-z'], root)
    return sorted(set(filter(None, (files + untracked).split('\0'))))


def docs_only(path):
    p = PurePosixPath(path)
    return (path in {'README.md', 'AGENTS.md', 'NOTICE', 'LICENSE-MIT', 'LICENSE-APACHE'}
            or (path.startswith(('docs/', 'notices/', 'skills/', 'tests/scenarios/')) and p.suffix == '.md')
            or (path.startswith('replica/') and p.suffix in {'.md', '.csv'}))


def workspace(meta, root=ROOT):
    members = set(meta['workspace_members'])
    packages = {p['name']: p for p in meta['packages'] if p['id'] in members}
    dirs = {name: Path(p['manifest_path']).resolve().parent.relative_to(root.resolve()).as_posix()
            for name, p in packages.items()}
    by_path = {str(Path(p['manifest_path']).resolve().parent): name for name, p in packages.items()}
    reverse = {name: set() for name in packages}
    for name, p in packages.items():
        for dep in p['dependencies']:
            # Metadata exposes the original package name even for aliases, and
            # path identity avoids confusing registry packages with members.
            target = by_path.get(str(Path(dep['path']).resolve())) if dep.get('path') else None
            if target:
                reverse[target].add(name)
    libs = {name for name, p in packages.items()
            if any(k in {'lib', 'rlib', 'dylib', 'cdylib', 'staticlib', 'proc-macro'}
                   for t in p['targets'] for k in t['kind'])}
    return packages, dirs, reverse, libs


def closure(changed, reverse):
    affected = set(changed)
    todo = list(changed)
    while todo:
        for name in reverse[todo.pop()] - affected:
            affected.add(name)
            todo.append(name)
    return affected


def result(mode, packages, libs, systems, integration, reasons):
    return dict(mode=mode, packages=packages, libs=libs,
                matrix={'include': [{'os': os, 'label': os} for os in systems]},
                integration=integration, reasons=reasons)


def full(reason):
    return result('full', '*', '*', OS, True, [reason])


def make_plan(files, meta, base_ref, force_full=False, root=ROOT):
    if force_full:
        return full('Full validation explicitly requested')
    if not base_ref.startswith('integration/'):
        return full('Targets outside integration require full validation')
    if all(docs_only(f) for f in files):
        return result('docs', [], [], [], False, ['Only allowlisted documentation changed'])
    packages, dirs, reverse, libs = workspace(meta, root)
    changed = set()
    for f in files:
        # Even crate-local manifests/build inputs can change graph or features.
        path = PurePosixPath(f)
        if path.name in {'Cargo.toml', 'Cargo.lock', 'build.rs'} or 'fixtures' in path.parts:
            return full('Build configuration, manifest or fixture changed: ' + f)
        owners = [name for name, d in dirs.items() if f.startswith(d + '/')]
        if owners:
            changed.add(max(owners, key=lambda name: len(dirs[name])))
        elif not docs_only(f):
            return full('Unknown or shared input changed: ' + f)
    affected = closure(changed, reverse)
    # Native behavior can be hidden in macros, external libraries or removed
    # code. Only explicitly portable leaf areas may use Linux alone.
    portable = {'name-match', 'stream-framing', 'sql-syntax'}
    native_consumers = COMMON | {'pyoxide-ide'} | {name for name in affected if name.endswith('-ui')}
    systems = [OS[0]] if changed <= portable and not affected & native_consumers else OS
    return result('fast', sorted(affected), sorted(affected & libs), systems,
                  bool(affected & DB), ['Affected packages and all reverse dependencies',
                  'Linux-only for explicitly portable crates' if systems == [OS[0]] else 'Native OS coverage retained'])


def load_plan(base, base_ref, force_full=False, root=ROOT):
    try:
        files = changed_files(base, root)
        if force_full or all(docs_only(f) for f in files) or not base_ref.startswith('integration/'):
            return make_plan(files, {}, base_ref, force_full, root)
        meta = json.loads(command(['cargo', 'metadata', '--format-version', '1', '--no-deps', '--locked'], root))
        return make_plan(files, meta, base_ref, force_full, root)
    except (subprocess.CalledProcessError, ValueError, KeyError, OSError) as exc:
        # Missing base/history or unresolvable metadata must never skip checks.
        return full('Scope could not be established: ' + type(exc).__name__)


def main():
    parser = argparse.ArgumentParser(description='Emit a conservative CI plan as JSON')
    parser.add_argument('--base', required=True)
    parser.add_argument('--base-ref', required=True)
    parser.add_argument('--force-full', action='store_true')
    args = parser.parse_args()
    print(json.dumps(load_plan(args.base, args.base_ref, args.force_full)))


if __name__ == '__main__':
    main()
