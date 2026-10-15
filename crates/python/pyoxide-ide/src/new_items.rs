//! Python's rows of *New* in the project tool window: *Python Package*
//! (`CreatePackageAction`: folders named by a dotted path, each with an
//! `__init__.py`) and *Python File* (`CreatePythonFileAction`: a Python
//! file, a unit test or a stub from the reference's internal file
//! templates).

use ui::icons::IconName;
use workbench::{NewItem, NewItemKind, NewTemplate};

/// The reference's internal template "Python Unit Test".
const UNIT_TEST: &str = "import unittest\n\nclass MyTestCase(unittest.TestCase):\n    def test_something(self):\n        self.assertEqual(True, False)  # add assertion here\n\nif __name__ == '__main__':\n    unittest.main()\n";

const PYTHON_FILE: IconName = IconName("python/file");

pub fn new_items() -> Vec<NewItem> {
    vec![
        NewItem {
            label: "Python Package",
            icon: IconName::PACKAGE,
            title: "New Python Package",
            kinds: vec![NewItemKind {
                label: "Package",
                icon: IconName::PACKAGE,
                // The template "Python Script" is empty.
                template: NewTemplate::Package {
                    marker: "__init__.py",
                    text: "",
                },
            }],
        },
        NewItem {
            label: "Python File",
            icon: PYTHON_FILE,
            title: "New Python File",
            kinds: vec![
                NewItemKind {
                    label: "Python file",
                    icon: PYTHON_FILE,
                    template: NewTemplate::File {
                        extension: "py",
                        text: "",
                    },
                },
                NewItemKind {
                    label: "Python unit test",
                    icon: PYTHON_FILE,
                    template: NewTemplate::File {
                        extension: "py",
                        text: UNIT_TEST,
                    },
                },
                NewItemKind {
                    label: "Python stub",
                    icon: PYTHON_FILE,
                    template: NewTemplate::File {
                        extension: "pyi",
                        text: "",
                    },
                },
            ],
        },
    ]
}
