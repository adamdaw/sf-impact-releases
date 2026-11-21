public class Circle extends BaseShape {
  private Decimal radius;

  public Circle() {
    this(1);
  }

  public Circle(Decimal radius) {
    this.radius = radius;
    this.label = 'circle';
  }

  public override Decimal area() {
    return MathUtil.round(MathUtil.PI_ISH * radius * radius, 2);
  }
}
