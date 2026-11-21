@IsTest
private class ShapeServiceTest {
  @IsTest
  static void totalsAreas() {
    Shape[] shapes = new List<Shape>{ new Circle(), new Square(2) };
    Assert.areEqual(7.14, ShapeService.totalArea(shapes));
  }

  static testMethod void describesShapes() {
    System.assertEquals(0, shapeservice.describeall(new List<Shape>()).size());
    // An array of shapes is a list: describe(Object), not describe(Shape).
    Assert.isNotNull(MathUtil.describe(new Shape[]{ new Circle() }));
  }

  private class FakeShape implements Shape {
    public Decimal area() {
      return 1;
    }

    public String describe(String prefix) {
      return prefix;
    }
  }
}
