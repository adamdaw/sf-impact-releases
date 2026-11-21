public class Invoice {
  public Invoice() {
  }

  public Invoice(String reference) {
  }

  public Decimal total() {
    return 0;
  }

  public Circle seal() {
    return new Circle();
  }

  public List<Circle> seals() {
    return new List<Circle>{ seal() };
  }
}
