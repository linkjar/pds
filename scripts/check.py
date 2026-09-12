#!/usr/bin/env python3
import importlib.util
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent

def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

class PinTests(unittest.TestCase):
    def test_release_guard_fails_closed_before_package_bootstrap(self):
        absent = load('release-guard').confirmed_absent
        self.assertFalse(absent(1, 'denied: requested access to the resource is denied'))
        self.assertFalse(absent(1, 'unauthorized: authentication required'))
        self.assertFalse(absent(1, 'lookup ghcr.io: host not found'))
        self.assertFalse(absent(0, ''))
        self.assertTrue(absent(1, 'manifest unknown\n'))
        self.assertTrue(absent(1, 'no such manifest: ghcr.io/linkjar/pds:unused\n'))

    def test_generated_dockerfile_is_current(self):
        pin = json.loads((ROOT / 'upstream.json').read_text())
        self.assertIsInstance(pin['revision'], int)
        self.assertGreater(pin['revision'], 0)
        self.assertRegex(pin['officialImage'], r'^ghcr.io/bluesky-social/pds@sha256:[0-9a-f]{64}$')
        expected = load('generate-dockerfile').generate(pin, (ROOT / 'docs/upstream.Dockerfile').read_text())
        self.assertEqual((ROOT / 'Dockerfile').read_text(), expected)

    def test_stable_tags_semver_and_annotated_tags(self):
        refs = '\n'.join([
            'tagobject refs/tags/@atproto/pds@0.5.9',
            'commit9 refs/tags/@atproto/pds@0.5.9^{}',
            'commit10 refs/tags/@atproto/pds@0.5.10',
            'ignored refs/tags/@atproto/pds@0.6.0-rc.1',
            'ignored refs/tags/@atproto/api@1.0.0',
        ])
        tags = load('check-upstream').releases(refs)
        self.assertEqual(tags['@atproto/pds@0.5.9'][1], 'commit9')
        self.assertEqual(max(tags, key=lambda t: tags[t][0]), '@atproto/pds@0.5.10')
        self.assertEqual(len(tags), 2)

if __name__ == '__main__':
    unittest.main()
