import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from scripts.run import SECTION_HEADINGS, SAFE_STANCE, extract_liko_stance


class ExtractLikoStanceTest(unittest.TestCase):
    def assert_valid_stance(self, stance: str) -> None:
        self.assertTrue(stance.strip())
        self.assertNotIn(stance, SECTION_HEADINGS)

    def test_normal_body_returns_first_signal_block(self):
        body = """本週訊號

一、遺產及贈與稅法修正草案仍待立法進度確認
財政部預告修正部分條文，值得高資產家庭檢視傳承安排節奏。

對應檢視面向
稅務身分

本週檢視動作
確認今年內贈與安排是否需要重排。
"""
        stance = extract_liko_stance(body)

        self.assertEqual(
            stance,
            "一、遺產及贈與稅法修正草案仍待立法進度確認 財政部預告修正部分條文，值得高資產家庭檢視傳承安排節奏。",
        )
        self.assert_valid_stance(stance)

    def test_blank_lines_after_heading(self):
        body = """本週訊號


海外所得申報季進入倒數，高資產家庭應重新確認申報義務。

對應檢視面向
稅務身分

本週檢視動作
確認海外所得資料是否完整。
"""
        stance = extract_liko_stance(body)

        self.assertEqual(stance, "海外所得申報季進入倒數，高資產家庭應重新確認申報義務。")
        self.assert_valid_stance(stance)

    def test_missing_heading_falls_back_to_non_heading_text(self):
        body = """# 本週不存在的標題

對應檢視面向

跨境家庭應先盤點稅務居民身分與申報義務。
"""
        stance = extract_liko_stance(body)

        self.assertEqual(stance, "本週不存在的標題")
        self.assert_valid_stance(stance)

    def test_empty_body_returns_safe_constant(self):
        stance = extract_liko_stance(" \n\n\t")

        self.assertEqual(stance, SAFE_STANCE)
        self.assert_valid_stance(stance)

    def test_never_returns_section_heading(self):
        for heading in SECTION_HEADINGS:
            self.assertNotEqual(extract_liko_stance(heading), heading)


if __name__ == "__main__":
    unittest.main()
