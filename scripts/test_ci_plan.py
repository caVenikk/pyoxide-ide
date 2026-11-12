import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('ci_plan', Path(__file__).with_name('ci-plan.py'))
plan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plan)


def metadata(root, definitions):
    packages = []
    for name, deps in definitions.items():
        packages.append(dict(id=name, name=name, manifest_path=str(root / name / 'Cargo.toml'),
                             targets=[{'kind': ['lib']}], dependencies=[
                                 dict(name=dep, rename='alias', kind=kind, path=str(root / dep))
                                 for dep, kind in deps]))
    return dict(workspace_members=list(definitions), packages=packages)


class PlannerTests(unittest.TestCase):
    def setUp(self):
        self.root = Path('/workspace')
        self.meta = metadata(self.root, {'name-match': [], 'consumer': [('name-match', 'build')],
                                        'app': [('consumer', 'dev')], 'other': []})

    def make(self, files, ref='integration/batch', force=False):
        return plan.make_plan(files, self.meta, ref, force, self.root)

    def test_alias_build_dev_transitive_closure(self):
        result = self.make(['name-match/src/lib.rs'])
        self.assertEqual(result['packages'], ['app', 'consumer', 'name-match'])
        self.assertEqual(result['libs'], result['packages'])
        self.assertEqual(result['matrix']['include'], [{'os': 'ubuntu-latest', 'label': 'ubuntu-latest'}])

    def test_portable_change_with_native_consumer_keeps_all_os(self):
        meta = metadata(self.root, {'name-match': [], 'ui': [('name-match', None)]})
        result = plan.make_plan(['name-match/src/lib.rs'], meta, 'integration/batch', root=self.root)
        self.assertEqual(len(result['matrix']['include']), 3)

    def test_main_and_unknown_target_always_full_even_docs(self):
        for ref in ['main', 'release/foo', '', 'integration']:
            self.assertEqual(self.make(['docs/guide.md'], ref)['mode'], 'full')

    def test_docs_allowlist_cannot_hide_code_or_build_inputs(self):
        self.assertEqual(self.make(['docs/guide.md', 'replica/features/feature.csv'])['mode'], 'docs')
        for path in ['scripts/check.sh', '.github/workflows/ci.yml', 'vendor/lib.rs',
                     'docs/fixture.rs', 'name-match/Cargo.toml', 'name-match/build.rs',
                     'Cargo.lock', 'tests/fixtures/example.md', 'removed-crate/src/lib.rs',
                     'name-match/fixtures/input.txt']:
            with self.subTest(path=path):
                self.assertEqual(self.make([path])['mode'], 'full')

    def test_crate_markdown_is_not_docs(self):
        self.assertEqual(self.make(['name-match/README.md'])['mode'], 'fast')

    def test_force_full_overrides_docs(self):
        result = self.make(['docs/guide.md'], force=True)
        self.assertEqual(result['mode'], 'full')
        self.assertTrue(result['integration'])
        self.assertEqual(len(result['matrix']['include']), 3)

    def test_common_crate_keeps_native_coverage_without_unrelated_packages(self):
        meta = metadata(self.root, {'process': [], 'consumer': [('process', 'build')],
                                    'app': [('consumer', 'dev')], 'db-core': []})
        result = plan.make_plan(['process/src/lib.rs'], meta, 'integration/batch', root=self.root)
        self.assertEqual(result['mode'], 'fast')
        self.assertEqual(result['packages'], ['app', 'consumer', 'process'])
        self.assertEqual(len(result['matrix']['include']), 3)
        self.assertFalse(result['integration'])

    def test_common_crate_with_db_consumer_runs_database_integration(self):
        meta = metadata(self.root, {'process': [], 'db-service': [('process', None)]})
        result = plan.make_plan(['process/src/lib.rs'], meta, 'integration/batch', root=self.root)
        self.assertEqual(result['mode'], 'fast')
        self.assertEqual(result['packages'], ['db-service', 'process'])
        self.assertEqual(len(result['matrix']['include']), 3)
        self.assertTrue(result['integration'])

    def test_workbench_retains_fixture_and_transitive_application_tests(self):
        meta = metadata(self.root, {'workbench': [],
                                    'workbench-fixture-module': [('workbench', 'dev')],
                                    'db-ui': [('workbench', None)],
                                    'pyoxide-ide': [('db-ui', None)], 'db-core': []})
        result = plan.make_plan(['workbench/src/menu.rs'], meta, 'integration/batch', root=self.root)
        self.assertEqual(result['mode'], 'fast')
        self.assertEqual(result['packages'], ['db-ui', 'pyoxide-ide', 'workbench', 'workbench-fixture-module'])
        self.assertEqual(result['libs'], result['packages'])
        self.assertEqual(len(result['matrix']['include']), 3)
        self.assertFalse(result['integration'])

    def test_common_crate_does_not_override_full_fallback(self):
        meta = metadata(self.root, {'process': []})
        for path in ['unknown-input', 'Cargo.lock', 'process/Cargo.toml', 'process/build.rs',
                     '.cargo/config.toml', 'process/fixtures/input.txt']:
            with self.subTest(path=path):
                result = plan.make_plan(['process/src/lib.rs', path], meta,
                                        'integration/batch', root=self.root)
                self.assertEqual(result['mode'], 'full')
                self.assertTrue(result['integration'])
        result = plan.make_plan(['process/src/lib.rs'], meta, 'main', root=self.root)
        self.assertEqual(result['mode'], 'full')

    def test_db_crates_retain_native_and_database_coverage(self):
        meta = metadata(self.root, {'db-core': [], 'db-service': [('db-core', None)], 'secrets': []})
        for name in ['db-core', 'secrets']:
            result = plan.make_plan([name + '/src/lib.rs'], meta, 'integration/batch', root=self.root)
            self.assertEqual(result['mode'], 'fast')
            self.assertEqual(len(result['matrix']['include']), 3)
            self.assertTrue(result['integration'])

    def test_missing_git_base_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(plan.load_plan('missing', 'integration/batch', root=Path(directory))['mode'], 'full')

    def test_exact_target_diff_rename_delete_and_untracked_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return plan.command(['git', *args], root).strip()
            git('init', '-q')
            git('config', 'user.name', 'CI Test')
            git('config', 'user.email', 'ci@example.invalid')
            (root / 'original.rs').write_text('original')
            git('add', '.')
            git('commit', '-qm', 'initial')
            initial = git('rev-parse', 'HEAD')
            (root / 'target-only.rs').write_text('target')
            git('add', '.')
            git('commit', '-qm', 'target advanced')
            target = git('rev-parse', 'HEAD')
            git('checkout', '-q', initial)
            git('mv', 'original.rs', 'renamed.rs')
            (root / 'untracked\nfile.rs').write_text('new')
            self.assertEqual(plan.changed_files(target, root),
                             ['original.rs', 'renamed.rs', 'target-only.rs', 'untracked\nfile.rs'])

    def test_check_script_uses_supplied_plan_and_rejects_malformed_scope(self):
        script = Path(__file__).with_name('check.sh').resolve()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cargo = root / 'cargo'
            cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$CALLS"\n')
            cargo.chmod(0o755)
            calls = root / 'calls'
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ['PATH'], CALLS=str(calls))
            file = root / 'plan.json'
            file.write_text(json.dumps(self.make(['name-match/src/lib.rs'])))
            result = subprocess.run(['bash', str(script), '--plan', str(file), 'build', 'test', 'doc'], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            commands = calls.read_text().splitlines()
            self.assertEqual(len(commands), 3)
            self.assertIn('-p app -p consumer -p name-match --profile ci --no-run', commands[0])
            self.assertIn('--profile ci', commands[1])
            self.assertEqual(commands[2], 'test --doc -p app -p consumer -p name-match')
            file.write_text('{"mode":"fast","packages":[],"libs":[]}')
            result = subprocess.run(['bash', str(script), '--plan', str(file), 'test'], env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(calls.read_text().splitlines(), commands)


if __name__ == '__main__':
    unittest.main()
