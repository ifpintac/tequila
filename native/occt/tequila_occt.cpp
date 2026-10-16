// tequila-occt: a STEP file read and divided by OpenCASCADE, for Tequila.
//
//   tequila-occt IN.step OUT.fsocc CHORD ANGLE
//
// Reads the file through OCCT's document layer (XDE), which keeps the
// assembly, every component's placement, product names and colours; divides
// each part once with BRepMesh at CHORD (a fraction of the part's size, as
// Tequila's own importer does) and ANGLE (radians); and writes what
// Tequila's importer needs to lay it out exactly as its pure-Rust reader
// does - products placing products and bodies, a body per solid, a mesh per
// colour - to OUT:
//
//   "FSOCC001", u64 length, JSON (UTF-8), then each mesh's arrays in order:
//   f32 positions xyz, f32 normals xyz, u32 triangle corners, f32 edge
//   segment end points xyz.
//
// Positions are in the file's own length unit, which the JSON names. It runs
// as a process of its own so that a file OCCT chokes on costs this process,
// not Tequila's window. Progress goes to standard output: "reading PERCENT"
// while OCCT turns the file into shapes, then "progress DONE TOTAL" in faces
// as they are divided.

#include <BRepBndLib.hxx>
#include <DESTEP_Parameters.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_ProgressIndicator.hxx>
#include <Message_ProgressScope.hxx>
#include <Poly_PolygonOnTriangulation.hxx>
#include <Poly_Triangulation.hxx>
#include <Quantity_Color.hxx>
#include <RWMesh_FaceIterator.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <STEPControl_Reader.hxx>
#include <Standard_Failure.hxx>
#include <TDF_Label.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Document.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <XCAFApp_Application.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <XCAFPrs_DocumentExplorer.hxx>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <string>
#include <vector>

namespace {

// A solid's faces, and each edge with the faces it bounds: the collections
// OCCT 8 asks for by name now its TopTools_* aliases are deprecated.
using FaceSet   = NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher>;
using EdgeUsers = NCollection_IndexedDataMap<TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;

// ---- JSON ------------------------------------------------------------------

std::string quoted(const std::string& text)
{
  std::string out = "\"";
  for (unsigned char c : text)
  {
    switch (c)
    {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20)
        {
          char buffer[8];
          std::snprintf(buffer, sizeof buffer, "\\u%04x", c);
          out += buffer;
        }
        else
        {
          out += static_cast<char>(c);
        }
    }
  }
  return out + "\"";
}

std::string number(double v)
{
  if (!std::isfinite(v))
  {
    return "0";
  }
  char buffer[32];
  std::snprintf(buffer, sizeof buffer, "%.9g", v);
  return buffer;
}

// ---- What is written --------------------------------------------------------

struct Mesh
{
  std::vector<float>    positions, normals, edges;
  std::vector<uint32_t> indices;
};

struct Group
{
  bool   coloured = false;
  double rgb[3]   = {0, 0, 0};
  int    mesh     = -1;
};

struct Body
{
  std::string        name;
  std::vector<Group> groups;
  int                faces = 0, lost = 0;
};

struct Product
{
  std::string          name;
  double               matrix[16];
  std::vector<int>     bodies;
  std::vector<Product> children;
};

// How far OCCT has got turning the file's entities into shapes, as
// "reading PERCENT" lines: on a big assembly that is most of the wait.
class Printer : public Message_ProgressIndicator
{
public:
  void Show(const Message_ProgressScope&, const bool isForce) override
  {
    const int percent = static_cast<int>(GetPosition() * 100.0);
    if (percent != myShown || isForce)
    {
      myShown = percent;
      std::printf("reading %d\n", percent);
      std::fflush(stdout);
    }
  }

private:
  int myShown = -1;
};

std::string name_of(const TDF_Label& label)
{
  occ::handle<TDataStd_Name> name;
  if (!label.IsNull() && label.FindAttribute(TDataStd_Name::GetID(), name))
  {
    return TCollection_AsciiString(name->Get()).ToCString();
  }
  return std::string();
}

void matrix_of(const TopLoc_Location& location, double scale, double out[16])
{
  const gp_Trsf t = location.Transformation();
  for (int r = 0; r < 3; ++r)
  {
    for (int c = 0; c < 4; ++c)
    {
      out[r * 4 + c] = t.Value(r + 1, c + 1) * (c == 3 ? scale : 1.0);
    }
  }
  out[12] = out[13] = out[14] = 0.0;
  out[15] = 1.0;
}

class Reader
{
public:
  Reader(double chord, double angle)
      : myChord(chord), myAngle(angle)
  {
  }

  bool read(const char* path, std::string& error)
  {
    occ::handle<XCAFApp_Application> app = XCAFApp_Application::GetApplication();
    app->NewDocument("MDTV-XCAF", myDoc);
    // Millimetres inside, as OCCT reads by default; the file's own unit is
    // what positions are written in.
    XCAFDoc_DocumentTool::SetLengthUnit(myDoc, 1.0, UnitsMethods_LengthUnit_Millimeter);
    STEPCAFControl_Reader reader;
    reader.SetColorMode(true);
    reader.SetNameMode(true);
    reader.SetLayerMode(false);
    reader.SetPropsMode(false);
    // The names of solids within a part as well as of parts: CAD packages
    // name bodies, and Tequila names its parts after them.
    DESTEP_Parameters parameters;
    parameters.InitFromStatic();
    parameters.ReadSubshapeNames = true;
    if (reader.ReadFile(path, parameters) != IFSelect_RetDone)
    {
      error = "OpenCASCADE could not read the file";
      return false;
    }
    NCollection_Sequence<TCollection_AsciiString> lengths, angles, solids;
    reader.ChangeReader().FileUnits(lengths, angles, solids);
    if (!lengths.IsEmpty())
    {
      myUnitName = lengths.First().ToCString();
    }
    myUnitMm = unit_in_mm(myUnitName);
    occ::handle<Printer> printer = new Printer();
    if (!reader.Transfer(myDoc, printer->Start()))
    {
      error = "OpenCASCADE could not turn the file's entities into shapes";
      return false;
    }
    myShapes = XCAFDoc_DocumentTool::ShapeTool(myDoc->Main());
    return true;
  }

  // Every part (a label that is not an assembly) divided once, then the
  // products laid out over them.
  void build()
  {
    NCollection_Sequence<TDF_Label> roots;
    myShapes->GetFreeShapes(roots);
    // The parts, and the size of the largest, which sets the floor of every
    // part's tolerance as Tequila's importer does.
    for (int i = 1; i <= roots.Length(); ++i)
    {
      collect_parts(roots.Value(i));
    }
    double model = 0.0;
    int    total = 0;
    for (auto& part : myParts)
    {
      Bnd_Box box;
      BRepBndLib::Add(myShapes->GetShape(part.label), box);
      part.size = box.IsVoid() ? 0.0 : std::sqrt(box.SquareExtent());
      model     = std::max(model, part.size);
      for (TopExp_Explorer f(myShapes->GetShape(part.label), TopAbs_FACE); f.More(); f.Next())
      {
        ++total;
      }
    }
    int done = 0;
    for (auto& part : myParts)
    {
      const double tolerance = std::max({part.size * myChord, model * myChord * 0.1, 1e-4});
      mesh_part(part, tolerance);
      for (TopExp_Explorer f(myShapes->GetShape(part.label), TopAbs_FACE); f.More(); f.Next())
      {
        ++done;
      }
      std::printf("progress %d %d\n", done, total);
      std::fflush(stdout);
    }
    for (int i = 1; i <= roots.Length(); ++i)
    {
      myProducts.push_back(product(roots.Value(i), TopLoc_Location()));
    }
  }

  bool write(const char* path, std::string& error) const
  {
    std::string json = "{\"unit\":" + quoted(myUnitName) + ",\"unit_mm\":" + number(myUnitMm);
    json += ",\"bodies\":[";
    for (size_t b = 0; b < myBodies.size(); ++b)
    {
      const Body& body = myBodies[b];
      json += (b ? "," : "") + std::string("{\"name\":") + quoted(body.name) + ",\"faces\":"
              + std::to_string(body.faces) + ",\"lost\":" + std::to_string(body.lost) + ",\"groups\":[";
      for (size_t g = 0; g < body.groups.size(); ++g)
      {
        const Group& group = body.groups[g];
        json += (g ? "," : "") + std::string("{\"mesh\":") + std::to_string(group.mesh) + ",\"colour\":";
        json += group.coloured ? "[" + number(group.rgb[0]) + "," + number(group.rgb[1]) + "," + number(group.rgb[2]) + "]"
                               : std::string("null");
        json += "}";
      }
      json += "]}";
    }
    json += "],\"products\":[";
    for (size_t p = 0; p < myProducts.size(); ++p)
    {
      json += (p ? "," : "") + product_json(myProducts[p]);
    }
    json += "],\"meshes\":[";
    for (size_t m = 0; m < myMeshes.size(); ++m)
    {
      const Mesh& mesh = myMeshes[m];
      json += (m ? "," : "") + std::string("{\"vertices\":") + std::to_string(mesh.positions.size() / 3)
              + ",\"triangles\":" + std::to_string(mesh.indices.size() / 3)
              + ",\"edge_points\":" + std::to_string(mesh.edges.size() / 3) + "}";
    }
    json += "]}";

    FILE* file = std::fopen(path, "wb");
    if (!file)
    {
      error = std::string("could not write ") + path;
      return false;
    }
    std::fwrite("FSOCC001", 1, 8, file);
    const uint64_t length = json.size();
    std::fwrite(&length, sizeof length, 1, file);
    std::fwrite(json.data(), 1, json.size(), file);
    for (const Mesh& mesh : myMeshes)
    {
      std::fwrite(mesh.positions.data(), sizeof(float), mesh.positions.size(), file);
      std::fwrite(mesh.normals.data(), sizeof(float), mesh.normals.size(), file);
      std::fwrite(mesh.indices.data(), sizeof(uint32_t), mesh.indices.size(), file);
      std::fwrite(mesh.edges.data(), sizeof(float), mesh.edges.size(), file);
    }
    const bool ok = std::ferror(file) == 0;
    std::fclose(file);
    if (!ok)
    {
      error = std::string("could not write all of ") + path;
    }
    return ok;
  }

private:
  struct Part
  {
    TDF_Label        label;
    double           size = 0.0;
    std::vector<int> bodies;
  };

  static double unit_in_mm(const std::string& name)
  {
    std::string upper = name;
    std::transform(upper.begin(), upper.end(), upper.begin(), [](unsigned char c) { return (char)std::toupper(c); });
    if (upper.find("MILLI") != std::string::npos || upper == "MM") return 1.0;
    if (upper.find("CENTI") != std::string::npos || upper == "CM") return 10.0;
    if (upper.find("INCH") != std::string::npos || upper == "IN") return 25.4;
    if (upper.find("FOOT") != std::string::npos || upper.find("FEET") != std::string::npos || upper == "FT") return 304.8;
    if (upper.find("METRE") != std::string::npos || upper.find("METER") != std::string::npos || upper == "M") return 1000.0;
    return 1.0;
  }

  // The label a component or a free shape stands for.
  TDF_Label referred(const TDF_Label& label) const
  {
    TDF_Label target;
    if (XCAFDoc_ShapeTool::IsReference(label) && XCAFDoc_ShapeTool::GetReferredShape(label, target))
    {
      return target;
    }
    return label;
  }

  void collect_parts(const TDF_Label& label)
  {
    const TDF_Label shape = referred(label);
    if (XCAFDoc_ShapeTool::IsAssembly(shape))
    {
      NCollection_Sequence<TDF_Label> components;
      XCAFDoc_ShapeTool::GetComponents(shape, components);
      for (int i = 1; i <= components.Length(); ++i)
      {
        collect_parts(components.Value(i));
      }
      return;
    }
    for (const Part& part : myParts)
    {
      if (part.label == shape)
      {
        return;
      }
    }
    Part part;
    part.label = shape;
    myParts.push_back(part);
  }

  Part* part_for(const TDF_Label& shape)
  {
    for (Part& part : myParts)
    {
      if (part.label == shape)
      {
        return &part;
      }
    }
    return nullptr;
  }

  // A part divided, its faces grouped into bodies (one per solid; faces in
  // no solid make one more) and, within a body, by colour.
  void mesh_part(Part& part, double tolerance)
  {
    const TopoDS_Shape shape = myShapes->GetShape(part.label);
    try
    {
      BRepMesh_IncrementalMesh mesher(shape, tolerance, false, myAngle, true);
    }
    catch (const Standard_Failure&)
    {
      // Faces left without a triangulation are counted as lost below.
    }

    std::vector<FaceSet>     solids;
    std::vector<std::string> solid_names;
    for (TopExp_Explorer s(shape, TopAbs_SOLID); s.More(); s.Next())
    {
      FaceSet faces;
      TopExp::MapShapes(s.Current(), TopAbs_FACE, faces);
      solids.push_back(faces);
      TDF_Label sub;
      solid_names.push_back(myShapes->FindSubShape(part.label, s.Current(), sub) ? name_of(sub) : std::string());
    }
    const std::string part_name = name_of(part.label);
    const size_t      first     = myBodies.size();
    const size_t      count     = solids.size() + 1;
    for (size_t b = 0; b < count; ++b)
    {
      Body body;
      if (b < solids.size())
      {
        body.name = !solid_names[b].empty() ? solid_names[b]
                    : solids.size() == 1    ? part_name
                                            : part_name + " " + std::to_string(b + 1);
      }
      else
      {
        body.name = part_name.empty() ? "Surfaces" : part_name;
      }
      myBodies.push_back(body);
    }
    auto body_of = [&](const TopoDS_Face& face) {
      for (size_t b = 0; b < solids.size(); ++b)
      {
        if (solids[b].Contains(face))
        {
          return b;
        }
      }
      return solids.size();
    };

    // Lost faces: those the mesher left without triangles.
    for (TopExp_Explorer f(shape, TopAbs_FACE); f.More(); f.Next())
    {
      const TopoDS_Face face = TopoDS::Face(f.Current());
      Body&             body = myBodies[first + body_of(face)];
      ++body.faces;
      TopLoc_Location location;
      const occ::handle<Poly_Triangulation> triangles = BRep_Tool::Triangulation(face, location);
      if (triangles.IsNull() || triangles->NbTriangles() == 0)
      {
        ++body.lost;
      }
    }

    for (RWMesh_FaceIterator face(part.label, TopLoc_Location(), true); face.More(); face.Next())
    {
      if (face.IsEmptyMesh())
      {
        continue;
      }
      Body& body = myBodies[first + body_of(face.Face())];
      double rgb[3] = {0, 0, 0};
      const bool coloured = face.HasFaceColor();
      if (coloured)
      {
        face.FaceColor().GetRGB().Values(rgb[0], rgb[1], rgb[2], Quantity_TOC_sRGB);
      }
      Group* group = nullptr;
      for (Group& g : body.groups)
      {
        if (g.coloured == coloured && (!coloured || (std::fabs(g.rgb[0] - rgb[0]) < 1e-6 && std::fabs(g.rgb[1] - rgb[1]) < 1e-6 && std::fabs(g.rgb[2] - rgb[2]) < 1e-6)))
        {
          group = &g;
          break;
        }
      }
      if (!group)
      {
        Group g;
        g.coloured = coloured;
        std::copy(rgb, rgb + 3, g.rgb);
        g.mesh = (int)myMeshes.size();
        myMeshes.emplace_back();
        body.groups.push_back(g);
        group = &body.groups.back();
      }
      Mesh&          mesh  = myMeshes[group->mesh];
      const uint32_t start = (uint32_t)(mesh.positions.size() / 3);
      const double   scale = 1.0 / myUnitMm;
      for (int n = face.NodeLower(); n <= face.NodeUpper(); ++n)
      {
        const gp_Pnt p = face.NodeTransformed(n);
        const gp_Dir d = face.NormalTransformed(n);
        mesh.positions.insert(mesh.positions.end(), {(float)(p.X() * scale), (float)(p.Y() * scale), (float)(p.Z() * scale)});
        mesh.normals.insert(mesh.normals.end(), {(float)d.X(), (float)d.Y(), (float)d.Z()});
      }
      for (int t = face.ElemLower(); t <= face.ElemUpper(); ++t)
      {
        int a, b, c;
        face.TriangleOriented(t).Get(a, b, c);
        const int lower = face.NodeLower();
        mesh.indices.insert(mesh.indices.end(), {start + (uint32_t)(a - lower), start + (uint32_t)(b - lower), start + (uint32_t)(c - lower)});
      }
    }

    // The B-rep's edges, as the mesher divided them, into each body's first
    // mesh: what the viewport outlines. Seams and degenerate edges are left
    // out, as CAD packages do.
    EdgeUsers users;
    TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, users);
    const double scale = 1.0 / myUnitMm;
    for (int e = 1; e <= users.Extent(); ++e)
    {
      const TopoDS_Edge edge = TopoDS::Edge(users.FindKey(e));
      const auto&       faces = users.FindFromIndex(e);
      if (BRep_Tool::Degenerated(edge) || faces.IsEmpty())
      {
        continue;
      }
      const TopoDS_Face face = TopoDS::Face(faces.First());
      if (BRep_Tool::IsClosed(edge, face))
      {
        continue;
      }
      TopLoc_Location location;
      const occ::handle<Poly_Triangulation> triangles = BRep_Tool::Triangulation(face, location);
      if (triangles.IsNull())
      {
        continue;
      }
      const occ::handle<Poly_PolygonOnTriangulation> polygon = BRep_Tool::PolygonOnTriangulation(edge, triangles, location);
      if (polygon.IsNull())
      {
        continue;
      }
      Body& body = myBodies[first + body_of(face)];
      if (body.groups.empty())
      {
        continue;
      }
      Mesh&        mesh  = myMeshes[body.groups.front().mesh];
      const gp_Trsf trsf = location.Transformation();
      const auto&  nodes = polygon->Nodes();
      for (int i = nodes.Lower(); i < nodes.Upper(); ++i)
      {
        for (int k : {nodes(i), nodes(i + 1)})
        {
          const gp_Pnt p = triangles->Node(k).Transformed(trsf);
          mesh.edges.insert(mesh.edges.end(), {(float)(p.X() * scale), (float)(p.Y() * scale), (float)(p.Z() * scale)});
        }
      }
    }

    for (size_t b = first; b < myBodies.size(); ++b)
    {
      if (!myBodies[b].groups.empty() || myBodies[b].faces > 0)
      {
        part.bodies.push_back((int)b);
      }
    }
  }

  Product product(const TDF_Label& label, const TopLoc_Location& location)
  {
    const TDF_Label shape = referred(label);
    Product         out;
    out.name = name_of(shape);
    if (out.name.empty())
    {
      out.name = name_of(label);
    }
    matrix_of(location, 1.0 / myUnitMm, out.matrix);
    if (XCAFDoc_ShapeTool::IsAssembly(shape))
    {
      NCollection_Sequence<TDF_Label> components;
      XCAFDoc_ShapeTool::GetComponents(shape, components);
      for (int i = 1; i <= components.Length(); ++i)
      {
        const TDF_Label component = components.Value(i);
        out.children.push_back(product(component, XCAFDoc_ShapeTool::GetLocation(component)));
      }
    }
    else if (Part* part = part_for(shape))
    {
      out.bodies = part->bodies;
    }
    return out;
  }

  std::string product_json(const Product& p) const
  {
    std::string json = "{\"name\":" + quoted(p.name) + ",\"matrix\":[";
    for (int i = 0; i < 16; ++i)
    {
      json += (i ? "," : "") + number(p.matrix[i]);
    }
    json += "],\"bodies\":[";
    for (size_t i = 0; i < p.bodies.size(); ++i)
    {
      json += (i ? "," : "") + std::to_string(p.bodies[i]);
    }
    json += "],\"children\":[";
    for (size_t i = 0; i < p.children.size(); ++i)
    {
      json += (i ? "," : "") + product_json(p.children[i]);
    }
    return json + "]}";
  }

  double                         myChord, myAngle;
  occ::handle<TDocStd_Document>  myDoc;
  occ::handle<XCAFDoc_ShapeTool> myShapes;
  std::string                    myUnitName = "MILLIMETRE";
  double                         myUnitMm   = 1.0;
  std::vector<Part>              myParts;
  std::vector<Body>              myBodies;
  std::vector<Mesh>              myMeshes;
  std::vector<Product>           myProducts;
};

} // namespace

int main(int argc, char** argv)
{
  if (argc < 5)
  {
    std::fprintf(stderr, "usage: tequila-occt IN.step OUT.fsocc CHORD ANGLE\n");
    return 2;
  }
  // OCCT's own messages would interleave with the progress lines.
  Message::DefaultMessenger()->RemovePrinters(STANDARD_TYPE(Message_Printer));
  try
  {
    Reader      reader(std::atof(argv[3]), std::atof(argv[4]));
    std::string error;
    if (!reader.read(argv[1], error))
    {
      std::fprintf(stderr, "%s\n", error.c_str());
      return 1;
    }
    reader.build();
    if (!reader.write(argv[2], error))
    {
      std::fprintf(stderr, "%s\n", error.c_str());
      return 1;
    }
    return 0;
  }
  catch (const Standard_Failure& failure)
  {
    std::fprintf(stderr, "OpenCASCADE failed: %s\n", failure.what());
    return 1;
  }
}
