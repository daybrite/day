import { winnerOf, turnOf, isDraw } from "../app.ts";

test("fresh board: X plays first, no winner", () => {
  expect(winnerOf("---------")).toBe("");
  expect(turnOf("---------")).toBe("X");
  expect(isDraw("---------")).toBe(false);
});

test("rows, columns, and diagonals win", () => {
  expect(winnerOf("XXX-OO---")).toBe("X");
  expect(winnerOf("O--OX-OXX" )).toBe("O");
  expect(winnerOf("X-OOXO--X")).toBe("X");
});

test("turns alternate", () => {
  expect(turnOf("X--------")).toBe("O");
  expect(turnOf("XO-------")).toBe("X");
});

test("a full board without a line is a draw", () => {
  expect(isDraw("XOXXOOOXX")).toBe(true);
  expect(isDraw("XOXXOOOX-")).toBe(false);
});
