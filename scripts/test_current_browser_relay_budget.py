"""Native budget evidence follows the saved policy, independently of Browser callers."""
import unittest

from check_current_browser_relay import EvidenceFailure, native_caller_budget_facts


class NativeBudgetEvidence(unittest.TestCase):
    def test_current_default_and_explicit_shorter_policy_are_accepted(self):
        for budget in (60_000, 12_000):
            with self.subTest(budget=budget):
                facts = native_caller_budget_facts(*({'callerBudgetMs': budget} for _ in range(3)))
                self.assertEqual(facts['callerBudgetMs'], budget)

    def test_reopen_and_member_cannot_silently_change_the_clock(self):
        for values in ((12_000, 60_000, 60_000), (12_000, 12_000, 60_000)):
            with self.subTest(values=values), self.assertRaises(EvidenceFailure):
                native_caller_budget_facts(*({'callerBudgetMs': v} for v in values))

    def test_unknown_or_boolean_budget_does_not_qualify(self):
        for value in (None, True, False, 0, -1):
            with self.subTest(value=value), self.assertRaises(EvidenceFailure):
                native_caller_budget_facts({'callerBudgetMs': value}, {'callerBudgetMs': value}, {'callerBudgetMs': value})
        for position in (1, 2):
            reports = [{'callerBudgetMs': 60_000} for _ in range(3)]
            reports[position] = {'callerBudgetMs': True}
            with self.subTest(position=position), self.assertRaises(EvidenceFailure):
                native_caller_budget_facts(*reports)


if __name__ == '__main__':
    unittest.main()
