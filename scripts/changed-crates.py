#!/usr/bin/env python3
# Local compatibility interface; Windows uses the complete affected closure.
import argparse
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location('ci_plan', Path(__file__).with_name('ci-plan.py'))
planner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(planner)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('base', nargs='?', default='origin/main')
    parser.add_argument('--base', dest='explicit_base')
    parser.add_argument('--lib', action='store_true')
    parser.add_argument('--windows', action='store_true')
    args = parser.parse_args()
    plan = planner.load_plan(args.explicit_base or args.base, 'integration/local')
    names = plan['libs' if args.lib else 'packages']
    print(names if names == '*' else '\n'.join(names), end='\n' if names else '')


if __name__ == '__main__':
    main()
