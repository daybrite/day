// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Included after the staged vector parser. No rasterization: Direct2D widens stroke
// centerlines into filled outlines and unions the independently painted SVG shapes.
struct IconGeometrySink : winrt::implements<IconGeometrySink, ID2D1SimplifiedGeometrySink> {
    WUXM::PathGeometry geometry;
    WUXM::PathFigure figure{nullptr};
    void STDMETHODCALLTYPE SetFillMode(D2D1_FILL_MODE mode) override {
        geometry.FillRule(mode == D2D1_FILL_MODE_ALTERNATE ? WUXM::FillRule::EvenOdd : WUXM::FillRule::Nonzero);
    }
    void STDMETHODCALLTYPE SetSegmentFlags(D2D1_PATH_SEGMENT) override {}
    void STDMETHODCALLTYPE BeginFigure(D2D1_POINT_2F point, D2D1_FIGURE_BEGIN begin) override {
        figure = WUXM::PathFigure();
        figure.StartPoint({point.x, point.y});
        figure.IsFilled(begin == D2D1_FIGURE_BEGIN_FILLED);
        geometry.Figures().Append(figure);
    }
    void STDMETHODCALLTYPE AddLines(D2D1_POINT_2F const* points, UINT32 count) override {
        for (UINT32 i = 0; i < count; ++i) {
            WUXM::LineSegment segment;
            segment.Point({points[i].x, points[i].y});
            figure.Segments().Append(segment);
        }
    }
    void STDMETHODCALLTYPE AddBeziers(D2D1_BEZIER_SEGMENT const* curves, UINT32 count) override {
        for (UINT32 i = 0; i < count; ++i) {
            WUXM::BezierSegment segment;
            segment.Point1({curves[i].point1.x, curves[i].point1.y});
            segment.Point2({curves[i].point2.x, curves[i].point2.y});
            segment.Point3({curves[i].point3.x, curves[i].point3.y});
            figure.Segments().Append(segment);
        }
    }
    void STDMETHODCALLTYPE EndFigure(D2D1_FIGURE_END end) override {
        figure.IsClosed(end == D2D1_FIGURE_END_CLOSED);
    }
    HRESULT STDMETHODCALLTYPE Close() override { return S_OK; }
};

static WUXM::Geometry icon_painted_geometry(Geom const& art) {
    winrt::com_ptr<ID2D1Factory> factory;
    winrt::check_hresult(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, factory.put()));
    winrt::com_ptr<ID2D1Geometry> painted;
    // A small source-space tolerance keeps the result smooth even in large icon slots.
    constexpr float tolerance = 0.01f;
    auto unite = [&](winrt::com_ptr<ID2D1Geometry> const& next) {
        if (!painted) { painted = next; return; }
        winrt::com_ptr<ID2D1PathGeometry> combined;
        winrt::check_hresult(factory->CreatePathGeometry(combined.put()));
        winrt::com_ptr<ID2D1GeometrySink> sink;
        winrt::check_hresult(combined->Open(sink.put()));
        winrt::check_hresult(painted->CombineWithGeometry(next.get(), D2D1_COMBINE_MODE_UNION,
            nullptr, tolerance, sink.get()));
        winrt::check_hresult(sink->Close());
        painted = combined.as<ID2D1Geometry>();
    };
    for (auto const& shape : art.shapes) {
        if (!shape.fill && !shape.stroke) continue;
        auto source = geometry_from_data(shape.data, shape.even_odd).try_as<WUXM::PathGeometry>();
        if (!source) return nullptr;
        winrt::com_ptr<ID2D1PathGeometry> path;
        winrt::check_hresult(factory->CreatePathGeometry(path.put()));
        winrt::com_ptr<ID2D1GeometrySink> sink;
        winrt::check_hresult(path->Open(sink.put()));
        sink->SetFillMode(shape.even_odd ? D2D1_FILL_MODE_ALTERNATE : D2D1_FILL_MODE_WINDING);
        for (auto const& figure : source.Figures()) {
            auto start = figure.StartPoint();
            sink->BeginFigure({start.X, start.Y}, D2D1_FIGURE_BEGIN_FILLED);
            for (auto const& segment : figure.Segments()) {
                if (auto line = segment.try_as<WUXM::LineSegment>()) {
                    auto point = line.Point();
                    sink->AddLine({point.X, point.Y});
                } else if (auto curve = segment.try_as<WUXM::BezierSegment>()) {
                    auto a = curve.Point1(), b = curve.Point2(), c = curve.Point3();
                    sink->AddBezier({{a.X, a.Y}, {b.X, b.Y}, {c.X, c.Y}});
                }
            }
            sink->EndFigure(figure.IsClosed() ? D2D1_FIGURE_END_CLOSED : D2D1_FIGURE_END_OPEN);
        }
        winrt::check_hresult(sink->Close());
        if (shape.fill) unite(path.as<ID2D1Geometry>());
        if (shape.stroke && shape.stroke_width > 0) {
            auto properties = D2D1::StrokeStyleProperties();
            properties.startCap = properties.endCap = shape.cap == "Round" ? D2D1_CAP_STYLE_ROUND
                : shape.cap == "Square" ? D2D1_CAP_STYLE_SQUARE : D2D1_CAP_STYLE_FLAT;
            properties.lineJoin = shape.join == "Round" ? D2D1_LINE_JOIN_ROUND
                : shape.join == "Bevel" ? D2D1_LINE_JOIN_BEVEL : D2D1_LINE_JOIN_MITER;
            winrt::com_ptr<ID2D1StrokeStyle> style;
            winrt::check_hresult(factory->CreateStrokeStyle(properties, nullptr, 0, style.put()));
            winrt::com_ptr<ID2D1PathGeometry> outline;
            winrt::check_hresult(factory->CreatePathGeometry(outline.put()));
            winrt::com_ptr<ID2D1GeometrySink> outline_sink;
            winrt::check_hresult(outline->Open(outline_sink.put()));
            winrt::check_hresult(path->Widen(static_cast<float>(shape.stroke_width), style.get(),
                nullptr, tolerance, outline_sink.get()));
            winrt::check_hresult(outline_sink->Close());
            unite(outline.as<ID2D1Geometry>());
        }
    }
    if (!painted) return nullptr;
    auto sink = winrt::make_self<IconGeometrySink>();
    winrt::check_hresult(painted->Simplify(D2D1_GEOMETRY_SIMPLIFICATION_OPTION_CUBICS_AND_LINES,
        nullptr, tolerance, sink.get()));
    winrt::check_hresult(sink->Close());
    return sink->geometry;
}
