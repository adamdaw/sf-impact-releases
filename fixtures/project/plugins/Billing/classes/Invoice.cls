public class Invoice {
  public Invoice() {
  }

  public Invoice(String reference) {
  }

  public Decimal total() {
    return total(false);
  }

  public Decimal total(Boolean withTax) {
    return withTax ? 120 : 100;
  }

  public Square seal() {
    return new Square(1);
  }

  public List<Square> seals() {
    return new List<Square>{ seal() };
  }
}
