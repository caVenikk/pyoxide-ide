import copy
import importlib.util
import io
import json
import os
import subprocess
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch
import zipfile


spec = importlib.util.spec_from_file_location('ci_control', Path(__file__).with_name('ci-control.py'))
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class FingerprintTests(unittest.TestCase):
    def test_nested_manifest_changes_invalidate_warm_seed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q', directory], check=True)
            (root / 'crates' / 'sample').mkdir(parents=True)
            (root / 'Cargo.toml').write_text('[workspace]\n')
            nested = root / 'crates' / 'sample' / 'Cargo.toml'
            nested.write_text('[package]\nname = "sample"\n')
            doc = root / 'README.md'
            doc.write_text('initial docs\n')
            subprocess.run(['git', '-C', directory, 'add', '.'], check=True)
            before = Path.cwd()
            try:
                os.chdir(root)
                original = ci.fingerprint()
                doc.write_text('changed docs\n')
                self.assertEqual(original, ci.fingerprint())
                nested.write_text(nested.read_text() + 'version = "0.2.0"\n')
                self.assertNotEqual(original, ci.fingerprint())
            finally:
                os.chdir(before)


class GateTests(unittest.TestCase):
    def setUp(self):
        self.pr = {'number': 7, 'state': 'open', 'draft': False,
                   'head': {'sha': 'head', 'repo': {'full_name': 'owner/repo'}},
                   'base': {'sha': 'base', 'ref': 'main', 'repo': {'full_name': 'owner/repo'}}}
        self.plan = ci.full_plan() | {'event': 'pull_request', 'pr': ci.identity(self.pr)}
        self.needs = {name: {'result': result} for name, result in
                      [('scope', 'success'), ('check', 'success'), ('integration', 'success'),
                       ('progress', 'success'), ('binaries', 'skipped')]}
        self.jobs = [{'name': name, 'status': 'completed', 'conclusion': 'success'}
                     for name in [f'check ({os})' for os in ci.OS] + ['integration', 'progress']]

    def test_full_requires_each_platform(self):
        ci.validate_results(self.plan, self.needs, self.jobs)
        for job in self.jobs:
            for result in ['skipped', 'cancelled', 'failure', None]:
                with self.subTest(job=job['name'], result=result):
                    changed = copy.deepcopy(self.jobs)
                    changed[self.jobs.index(job)]['conclusion'] = result
                    with self.assertRaises(ValueError):
                        ci.validate_results(self.plan, self.needs, changed)
        with self.assertRaises(ValueError):
            ci.validate_results(self.plan, self.needs, self.jobs[:-1])
        with self.assertRaises(ValueError):
            ci.validate_results(self.plan, self.needs, self.jobs + self.jobs[:1])

    def test_aggregate_failures_never_count_as_success(self):
        for name in self.needs:
            for result in ['failure', 'cancelled', 'skipped']:
                if name == 'binaries' and result == 'skipped':
                    continue
                changed = copy.deepcopy(self.needs)
                changed[name]['result'] = result
                with self.subTest(name=name, result=result), self.assertRaises(ValueError):
                    ci.validate_results(self.plan, changed, self.jobs)

    def test_draft_docs_and_reuse_expect_real_skips(self):
        for mode in ['draft', 'docs']:
            plan = ci.skipped_plan(mode) | {'event': 'pull_request', 'pr': {'base_ref': 'integration/test'}}
            needs = copy.deepcopy(self.needs)
            needs['check']['result'] = needs['integration']['result'] = 'skipped'
            ci.validate_results(plan, needs, self.jobs[-1:])
            needs['check']['result'] = 'cancelled'
            with self.assertRaises(ValueError):
                ci.validate_results(plan, needs, self.jobs[-1:])

    def test_main_and_dispatch_never_accept_reduced_plans(self):
        for mode in ['fast', 'docs', 'reuse']:
            with self.assertRaises(ValueError):
                ci.validate_results(self.plan | {'mode': mode}, self.needs, self.jobs)
        for key, value in [('packages', ['one']), ('libs', []), ('integration', 'true')]:
            with self.assertRaises(ValueError):
                ci.validate_results(self.plan | {key: value}, self.needs, self.jobs)
        with self.assertRaises(ValueError):
            ci.validate_results(self.plan | {'event': 'workflow_dispatch', 'mode': 'reuse'},
                                self.needs, self.jobs)

    def test_fast_requires_exact_matrix(self):
        plan = copy.deepcopy(self.plan)
        plan['mode'] = 'fast'
        plan['pr']['base_ref'] = 'integration/test'
        plan['matrix']['include'] = plan['matrix']['include'][:1]
        plan['integration'] = False
        needs = copy.deepcopy(self.needs)
        needs['integration']['result'] = 'skipped'
        ci.validate_results(plan, needs, self.jobs[:1] + self.jobs[-1:])
        with self.assertRaises(ValueError):
            ci.validate_results(plan, needs, self.jobs[-1:])

    def test_base_head_retarget_closed_and_draft_transitions(self):
        ci.validate_live(self.plan, self.pr)
        mutations = [('head', 'sha', 'new'), ('base', 'sha', 'new'), ('base', 'ref', 'integration/new')]
        for outer, key, value in mutations:
            pr = copy.deepcopy(self.pr)
            pr[outer][key] = value
            with self.assertRaises(ValueError):
                ci.validate_live(self.plan, pr)
        for key, value in [('draft', True), ('state', 'closed')]:
            with self.assertRaises(ValueError):
                ci.validate_live(self.plan, self.pr | {key: value})
        draft = self.plan | {'mode': 'draft'}
        ci.validate_live(draft, self.pr | {'draft': True})
        with self.assertRaises(ValueError):
            ci.validate_live(draft, self.pr)  # #335: stale draft survivor


class ProofTests(unittest.TestCase):
    def setUp(self):
        self.plan = {'repository': 'owner/repo', 'tree': 'tree', 'config': 'config', 'rustc': 'rustc version'}
        self.pr = {'number': 7, 'head': {'sha': 'head'}}
        self.run = {'id': 12, 'run_attempt': 1, 'event': 'pull_request', 'status': 'completed',
                    'conclusion': 'success', 'path': ci.WORKFLOW, 'head_sha': 'head',
                    'repository': {'full_name': 'owner/repo'}, 'head_repository': {'full_name': 'owner/repo'},
                    'pull_requests': [{'number': 7, 'head': {'sha': 'head'}, 'base': {'ref': 'main'}}]}
        self.proof = self.plan | {'schema': 1, 'mode': 'full', 'workflow': ci.WORKFLOW,
                                 'run_id': 12, 'run_attempt': 1, 'commit': 'merge',
                                 'pr': {'number': 7, 'head': 'head', 'base': 'base', 'base_ref': 'main',
                                        'head_repo': 'owner/repo', 'base_repo': 'owner/repo'},
                                 'checks': [f'check ({os})' for os in ci.OS] + ['integration', 'progress']}
        self.commit = {'tree': {'sha': 'tree'}, 'parents': [{'sha': 'base'}, {'sha': 'head'}]}

    def validate(self, proof=None, run=None, commit=None):
        ci.validate_proof(proof or self.proof, run or self.run, self.pr, self.plan, commit or self.commit)

    def test_accepts_only_exact_full_evidence(self):
        self.validate()
        for key, value in [('schema', 2), ('schema', True), ('mode', 'fast'), ('mode', 'docs'), ('mode', 'draft'),
                           ('repository', 'fork/repo'), ('run_id', 13), ('run_attempt', 2),
                           ('tree', 'other'), ('config', 'other'), ('rustc', 'other'), ('checks', [])]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                self.validate(proof=self.proof | {key: value})
        for key, value in [('number', 8), ('head', 'other'), ('base_ref', 'integration/batch'),
                           ('head_repo', 'fork/repo'), ('base_repo', 'fork/repo')]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate(proof=self.proof | {'pr': self.proof['pr'] | {key: value}})

    def test_completed_successful_correct_workflow_run_only(self):
        for key, value in [('status', 'in_progress'), ('conclusion', 'failure'), ('conclusion', 'cancelled'),
                           ('event', 'push'), ('path', '.github/workflows/other.yml'),
                           ('head_sha', 'other'), ('pull_requests', [{'number': 8}])]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate(run=self.run | {key: value})

    def test_merged_run_may_have_empty_api_association(self):
        self.validate(run=self.run | {'pull_requests': []})
        with self.assertRaises(ValueError):
            self.validate(run=self.run | {'pull_requests': [], 'head_sha': 'different'})
        with self.assertRaises(ValueError):
            self.validate(run=self.run | {'pull_requests': [], 'head_repository': {'full_name': 'fork/repo'}})

    def test_actual_merge_commit_tree_and_parents(self):
        for commit in [{'tree': {'sha': 'other'}, 'parents': self.commit['parents']},
                       {'tree': {'sha': 'tree'}, 'parents': [{'sha': 'head'}]},
                       {'tree': {'sha': 'tree'}, 'parents': list(reversed(self.commit['parents']))}]:
            with self.assertRaises(ValueError):
                self.validate(commit=commit)

    def test_artifact_is_data_only_and_bounded(self):
        for files in [{'proof.json': json.dumps(self.proof)}, {'../proof.json': '{}'},
                      {'proof.json': '{}', 'run.sh': 'exit 0'}, {'proof.json': 'x' * 20000}]:
            raw = io.BytesIO()
            with zipfile.ZipFile(raw, 'w') as archive:
                for name, content in files.items():
                    archive.writestr(name, content)
            with patch.object(ci.subprocess, 'check_output', return_value=raw.getvalue()):
                artifact = {'id': 1, 'expired': False, 'size_in_bytes': len(raw.getvalue())}
                if list(files) == ['proof.json'] and len(files['proof.json']) < 16384:
                    self.assertEqual(ci.read_artifact('owner/repo', artifact), self.proof)
                else:
                    with self.assertRaises(ValueError):
                        ci.read_artifact('owner/repo', artifact)

    def test_pagination_collects_all_pages(self):
        with patch.object(ci, 'api', side_effect=[{'jobs': list(range(100))}, {'jobs': [100]}]):
            self.assertEqual(len(ci.pages('repos/example/actions/jobs', 'jobs')), 101)



class OrchestrationTests(unittest.TestCase):
    setUp = ProofTests.setUp

    def test_reuse_chain_is_bound_to_pushed_commit_and_pr(self):
        pr = {'number': 7, 'draft': False, 'merged': True, 'merged_at': '2026-01-01',
              'merge_commit_sha': 'pushed', 'base': {'ref': 'main', 'repo': {'full_name': 'owner/repo'}},
              'head': {'sha': 'head', 'repo': {'full_name': 'owner/repo'}}}
        plan = self.plan | {'commit': 'pushed'}
        artifact = {'id': 22, 'name': 'ci-full-proof-12-1'}
        with patch.object(ci, 'warm_cache_seed'), patch.object(ci, 'pages', side_effect=[
                [pr], [self.run], [artifact]]), patch.object(ci, 'live_pr', return_value=pr), \
                patch.object(ci, 'api', side_effect=[self.run, self.commit]), \
                patch.object(ci, 'read_artifact', return_value=self.proof):
            self.assertEqual(ci.reusable(plan)['artifact_id'], 22)
        for changed in [pr | {'merge_commit_sha': 'other'}, pr | {'merged_at': None},
                        pr | {'base': pr['base'] | {'ref': 'integration/batch'}},
                        pr | {'head': pr['head'] | {'repo': {'full_name': 'fork/repo'}}}]:
            with patch.object(ci, 'warm_cache_seed'), patch.object(ci, 'pages', return_value=[changed]):
                with self.assertRaises(ValueError):
                    ci.reusable(plan)
        for artifacts in [[], [artifact, artifact]]:
            with patch.object(ci, 'warm_cache_seed'), patch.object(ci, 'pages', side_effect=[
                    [pr], [self.run], artifacts]), patch.object(ci, 'live_pr', return_value=pr), \
                    patch.object(ci, 'api', return_value=self.run):
                with self.assertRaises(ValueError):
                    ci.reusable(plan)

    def test_warm_seed_is_recent_complete_matching_main(self):
        now = ci.datetime.now(ci.timezone.utc)
        run = self.run | {'event': 'push', 'head_branch': 'main', 'head_sha': 'main-sha',
                          'created_at': now.isoformat()}
        proof = self.proof | {'commit': 'main-sha'}
        artifact = {'id': 22, 'name': 'ci-main-cache-state-12-1'}
        with patch.object(ci, 'pages', side_effect=[[run], [artifact]]), \
                patch.object(ci, 'read_artifact', return_value=proof):
            ci.warm_cache_seed(self.plan)
        for changed in [run | {'event': 'pull_request'}, run | {'status': 'in_progress'},
                        run | {'head_branch': 'other'}, run | {'conclusion': 'cancelled'},
                        run | {'created_at': (now - ci.timedelta(days=7)).isoformat()}]:
            with patch.object(ci, 'pages', return_value=[changed]), self.assertRaises(ValueError):
                ci.warm_cache_seed(self.plan)
        for key in ['rustc', 'config', 'mode', 'commit', 'checks']:
            with patch.object(ci, 'pages', side_effect=[[run], [artifact]]), \
                    patch.object(ci, 'read_artifact', return_value=proof | {key: 'different'}), \
                    self.assertRaises(ValueError):
                ci.warm_cache_seed(self.plan)

    def test_latest_job_per_name_allows_failed_only_rerun(self):
        base = {'run_id': 12, 'head_sha': 'head', 'run_attempt': 1,
                'status': 'completed', 'conclusion': 'success'}
        linux = base | {'name': 'check (ubuntu-latest)', 'id': 1}
        failed = base | {'name': 'check (windows-latest)', 'id': 2, 'conclusion': 'failure'}
        rerun = failed | {'id': 3, 'run_attempt': 2, 'conclusion': 'success'}
        result = ci.latest_jobs([linux, failed, rerun], 12, 'head', 2)
        self.assertEqual(result, [linux, rerun])
        for key, value in [('run_id', 13), ('head_sha', 'other'), ('run_attempt', 3)]:
            with self.assertRaises(ValueError):
                ci.latest_jobs([linux | {key: value}], 12, 'head', 2)
        with self.assertRaises(ValueError):
            ci.latest_jobs([linux, linux | {'id': 4}], 12, 'head', 2)

    def test_scope_fallback_dispatch_and_live_checkout(self):
        import os
        import tempfile
        pr = {'number': 7, 'draft': False, 'state': 'open',
              'head': {'sha': 'head', 'repo': {'full_name': 'owner/repo'}},
              'base': {'sha': 'base', 'ref': 'main', 'repo': {'full_name': 'owner/repo'}}}
        def git(*args):
            return {'HEAD': 'merge', 'HEAD^{tree}': 'tree', 'HEAD^1': 'base', 'HEAD^2': 'head'}[args[-1]]
        def command(*args):
            return 'rustc version' if args[0] == 'rustc' else json.dumps(ci.full_plan())
        with tempfile.TemporaryDirectory() as temporary:
            output = str(Path(temporary) / 'output')
            env = {'EVENT': 'push', 'GITHUB_REPOSITORY': 'owner/repo',
                   'GITHUB_REF': 'refs/heads/main', 'GITHUB_OUTPUT': output, 'PULL_REQUEST': '7'}
            previous = os.getcwd()
            os.chdir(temporary)
            try:
                with patch.dict(os.environ, env), patch.object(ci, 'git', side_effect=git), \
                        patch.object(ci, 'fingerprint', return_value='config'), \
                        patch.object(ci, 'command', side_effect=command), \
                        patch.object(ci, 'reusable', side_effect=ValueError('API unavailable')) as reuse:
                    ci.scope()
                    self.assertEqual(json.loads(Path('.ci-plan.json').read_text())['mode'], 'full')
                    reuse.reset_mock()
                    os.environ['EVENT'] = 'workflow_dispatch'
                    ci.scope()
                    reuse.assert_not_called()
                    os.environ['EVENT'] = 'pull_request'
                    with patch.object(ci, 'live_pr', return_value=pr):
                        ci.scope()
                        self.assertEqual(json.loads(Path('.ci-plan.json').read_text())['mode'], 'full')
                    with patch.object(ci, 'live_pr', return_value=pr | {'draft': True}):
                        ci.scope()
                        self.assertEqual(json.loads(Path('.ci-plan.json').read_text())['mode'], 'draft')
                    with patch.object(ci, 'live_pr', return_value=pr | {'head': pr['head'] | {'sha': 'new'}}):
                        with self.assertRaises(ValueError):
                            ci.scope()
            finally:
                os.chdir(previous)

if __name__ == '__main__':
    unittest.main()
