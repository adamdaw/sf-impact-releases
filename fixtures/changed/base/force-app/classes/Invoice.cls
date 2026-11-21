public with sharing class Invoice {
    public static final Decimal RATE = 0.2;

    public Decimal amount;

    public Invoice(Decimal amount) {
        this.amount = amount;
    }

    public Decimal total() {
        return amount + tax();
    }

    public Decimal tax() {
        return amount * RATE;
    }

    public Decimal round(Decimal value) {
        return value.setScale(2);
    }

    public class Line {
        public Integer quantity;

        public Decimal amount() {
            return quantity * 1.0;
        }
    }
}
