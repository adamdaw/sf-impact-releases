public class FlowInvoiceAction {
    @InvocableMethod public static void run(List<Id> ids) {}
    @AuraEnabled public static String caption(Id recordId) { return 'Invoice'; }
}
