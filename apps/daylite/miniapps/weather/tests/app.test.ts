import { weatherKey, formatTemp } from "../app.ts";

test("weather codes map to Fluent keys", () => {
  expect(weatherKey(0)).toBe("wx-clear");
  expect(weatherKey(2)).toBe("wx-partly");
  expect(weatherKey(63)).toBe("wx-rain");
  expect(weatherKey(75)).toBe("wx-snow");
  expect(weatherKey(96)).toBe("wx-thunder");
});

test("i18n resolves through the shipped .ftl catalogs", () => {
  expect(t("wx-rain")).toBe("Rain");
  expect(day.i18n.t("wind", { n: 12 })).toBe("Wind 12 km/h");
  expect(t("no-such-key")).toBe("no-such-key");
});

test("temperatures round to whole degrees", () => {
  expect(formatTemp(21.4)).toBe("21°C");
  expect(formatTemp(9.8)).toBe("10°C");
  expect(formatTemp(-3.6)).toBe("-4°C");
});

test("network stays rejected in tests", () => {
  expect(day.can("NETWORK")).toBe(false);
});
