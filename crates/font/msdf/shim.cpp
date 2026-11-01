// C ABI over msdfgen's core, for font::bake. One call = one glyph: outline
// commands in, MSDF cell + quad out.
#include "msdfgen.h"
#include <algorithm>

using namespace msdfgen;

// ops: 0 move_to(1 pt), 1 line_to(1 pt), 2 quad_to(2 pts), 3 curve_to(3 pts), 4
// close; pts: x, y pairs in font units. out: cell * cell * 3 floats, top row
// first (0.5 = the outline), the glyph centered with range_px / 2 free px on
// every side. quad: left, bottom, right, top of the whole cell in font units (y
// up). Returns 0 on success, 1 for a glyph with no outline (a space), 2 for an
// invalid shape.
extern "C" int msdf_glyph(const unsigned char *ops, size_t n_ops,
                          const double *pts, unsigned cell, double range_px,
                          float *out, double *quad) {
  Shape shape;
  Contour *contour = nullptr;
  Point2 start, cur;
  auto pt = [&]() -> Point2 {
    Point2 p(pts[0], pts[1]);
    pts += 2;
    return p;
  };
  auto close = [&] {
    if (contour && !(cur == start))
      contour->addEdge(EdgeHolder(cur, start));
    cur = start;
  };
  for (size_t i = 0; i < n_ops; i++) {
    switch (ops[i]) {
    case 0:
      close();
      contour = &shape.addContour();
      start = cur = pt();
      break;
    case 1: {
      Point2 p = pt();
      if (contour && !(p == cur))
        contour->addEdge(EdgeHolder(cur, p));
      cur = p;
      break;
    }
    case 2: {
      Point2 c = pt(), p = pt();
      if (contour)
        contour->addEdge(EdgeHolder(cur, c, p));
      cur = p;
      break;
    }
    case 3: {
      Point2 c1 = pt(), c2 = pt(), p = pt();
      if (contour)
        contour->addEdge(EdgeHolder(cur, c1, c2, p));
      cur = p;
      break;
    }
    default:
      close();
    }
  }
  close();

  shape.contours.erase(
      std::remove_if(shape.contours.begin(), shape.contours.end(),
                     [](const Contour &c) { return c.edges.empty(); }),
      shape.contours.end());
  if (shape.contours.empty())
    return 1;
  if (!shape.validate())
    return 2;
  shape.normalize();
  edgeColoringSimple(shape, 3.0);

  Shape::Bounds b = shape.getBounds();
  double size = std::max(b.r - b.l, b.t - b.b);
  if (!(size > 0))
    return 1;
  double scale = (cell - range_px) /
                 size; // px per font unit, range_px / 2 of margin each side
  double half = cell / 2.0 / scale; // half the cell in font units
  double cx = (b.l + b.r) / 2, cy = (b.b + b.t) / 2;
  quad[0] = cx - half, quad[1] = cy - half, quad[2] = cx + half,
  quad[3] = cy + half;

  BitmapSection<float, 3> bmp(
      out, cell, cell,
      Y_DOWNWARD); // generateMSDF reorients it to the shape's Y axis
  SDFTransformation t(Projection(Vector2(scale), Vector2(half - cx, half - cy)),
                      Range(range_px / scale));
  generateMSDF(bmp, shape, t);
  return 0;
}
