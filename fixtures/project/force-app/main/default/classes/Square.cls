public class Square extends BaseShape {
  public Decimal side;

  public Square(Decimal side) {
    this.side = side;
    label = 'square';
  }

  public override Decimal area() {
    return side * side;
  }

  public override String describe(String prefix) {
    return super.describe(prefix).toUpperCase();
  }
}
