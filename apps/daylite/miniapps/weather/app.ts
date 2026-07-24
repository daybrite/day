// Weather — open-meteo over day.net.fetch (docs/lite.md §7): geocode the city, fetch
// current conditions, render reactively. All state lives in signals; the async flow is
// plain await (promises resolve on day's main thread).

const city = signal("Berlin");
const status = signal("");
const place = signal("");
const temp = signal("");
const wind = signal("");
const sky = signal("");

// WMO weather interpretation codes → Fluent key (the unit-testable core).
export function weatherKey(code: number): string {
  if (code === 0) return "wx-clear";
  if (code === 1 || code === 2) return "wx-partly";
  if (code === 3) return "wx-overcast";
  if (code === 45 || code === 48) return "wx-fog";
  if (code >= 51 && code <= 57) return "wx-drizzle";
  if (code >= 61 && code <= 67) return "wx-rain";
  if (code >= 71 && code <= 77) return "wx-snow";
  if (code >= 80 && code <= 82) return "wx-showers";
  if (code === 85 || code === 86) return "wx-snow-showers";
  if (code >= 95) return "wx-thunder";
  return "wx-mixed";
}

export function formatTemp(celsius: number): string {
  return `${Math.round(celsius)}°C`;
}

async function refresh(): Promise<void> {
  const q = city.get().trim();
  if (q.length === 0) return;
  status.set(t("status-looking", { q }));
  place.set("");
  try {
    const geo = await day.net.fetch(
      "https://geocoding-api.open-meteo.com/v1/search?count=1&name=" + encodeURIComponent(q),
    );
    const hits = geo.json().results;
    if (!hits || hits.length === 0) {
      status.set(t("status-none", { q }));
      return;
    }
    const spot = hits[0];
    status.set(t("status-fetching"));
    const wx = await day.net.fetch(
      "https://api.open-meteo.com/v1/forecast?current_weather=true" +
        `&latitude=${spot.latitude}&longitude=${spot.longitude}`,
    );
    const current = wx.json().current_weather;
    place.set(spot.name + (spot.country ? ", " + spot.country : ""));
    temp.set(formatTemp(current.temperature));
    wind.set(t("wind", { n: Math.round(current.windspeed) }));
    sky.set(t(weatherKey(current.weathercode)));
    status.set("");
  } catch (e) {
    status.set(t("status-error", { e: String(e) }));
  }
}

App({});

page("home", () =>
  column(
    label(() => t("title")).font("large_title"),
    row(
      text_field(city).placeholder(t("city-placeholder")).on_submit(refresh).id("wx-city"),
      button(() => t("go")).action(refresh).id("wx-go"),
    ).spacing(8),
    when(
      () => status.get().length > 0 || place.get().length === 0,
      () => label(() => (status.get().length > 0 ? status.get() : t("prompt"))).font("footnote"),
    ),
    when(
      () => place.get().length > 0,
      () =>
        column(
          label(() => place.get()).font("title2"),
          label(() => temp.get()).font("large_title"),
          label(() => sky.get()).font("headline"),
          label(() => wind.get()).font("callout"),
        )
          .spacing(6)
          .padding(12)
          .background("#ffffff")
          .corner_radius(12)
          .id("wx-card"),
    ),
  )
    .spacing(12)
    .padding(16)
    .id("wx-root"),
);
