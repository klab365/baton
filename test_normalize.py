import unittest

from normalize import normalize


class NormalizeTests(unittest.TestCase):
    def test_trims_leading_and_trailing_spaces(self):
        self.assertEqual(normalize("  hello world  "), "hello world")

    def test_collapses_consecutive_spaces(self):
        self.assertEqual(normalize("hello   world"), "hello world")

    def test_preserves_single_spaces(self):
        self.assertEqual(normalize("hello world"), "hello world")

    def test_empty_text(self):
        self.assertEqual(normalize(""), "")

    def test_text_containing_only_spaces(self):
        self.assertEqual(normalize("   "), "")

    def test_single_space(self):
        self.assertEqual(normalize(" "), "")


if __name__ == "__main__":
    unittest.main()
