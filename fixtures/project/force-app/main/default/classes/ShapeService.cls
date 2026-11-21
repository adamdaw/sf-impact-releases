public with sharing class ShapeService {
  private static final Map<String, Shape> REGISTRY = new Map<String, Shape>();

  @AuraEnabled(cacheable=true)
  public static List<String> describeAll(List<Shape> shapes) {
    List<String> lines = new List<String>();
    for (Shape shape : shapes) {
      lines.add(MathUtil.describe(shape));
      // Apex ignores case; so must the match.
      lines.add(MATHUTIL.DESCRIBE(shape.area()));
    }
    return lines;
  }

  public static Decimal totalArea(Shape[] shapes) {
    Decimal total = 0;
    for (Integer i = 0; i < shapes.size(); i++) {
      total += shapes[i].area();
    }
    return MathUtil.clamp(total, 0, 1000);
  }

  public static Decimal firstArea(System.List<Shape> shapes) {
    // A collection may be written with its namespace; it is the same list.
    return shapes.get(0).area();
  }

  public static Decimal lookupArea(String name) {
    // A Map returns its value type: this is Shape.area().
    return MathUtil.round(REGISTRY.get(name).area());
  }

  public static Object clampAny(String json) {
    // The first argument's type is unknown: both overloads of clamp fit.
    return MathUtil.clamp(JSON.deserializeUntyped(json), 0, 10);
  }

  public static Shape make(String kind, Decimal size) {
    if (kind == 'circle') {
      return new Circle(size);
    }
    // A class named in a string: only the textual tier sees `Square` here.
    Shape made = (Shape) Type.forName('Square').newInstance();
    return made instanceof BaseShape ? made : new Square(size);
  }

  public static MathUtil.Mode mode() {
    return MathUtil.Mode.UP;
  }

  public static Money price(Shape shape) {
    npsp.Pricing.Quote quote = npsp.Pricing.quote(shape.area());
    // A call on an installed package's type is external. What it returns
    // is not known here, so the call after it is to a method of unknown type.
    quote.revise().discount();
    return new Money(quote.amount);
  }
}
