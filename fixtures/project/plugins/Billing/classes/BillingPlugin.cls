public class BillingPlugin {
  public Decimal run(Invoice invoice) {
    return invoice.total() + new Invoice().total(true);
  }

  public Decimal sealArea() {
    // The two Invoice classes disagree about what seal() returns, so the
    // call to area() is to one of two methods and the row lists both.
    return new Invoice('INV-1').seal().area();
  }

  public Decimal firstSealArea() {
    // The same through a list: its elements are circles in one Invoice
    // class and squares in the other.
    return new Invoice('INV-2').seals().get(0).area();
  }
}
