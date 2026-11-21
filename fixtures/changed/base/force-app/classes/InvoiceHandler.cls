public class InvoiceHandler {
    @TestVisible
    private void handle() {
        new Invoice(1).total();
    }
}
