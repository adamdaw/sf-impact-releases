public with sharing class Tariff {
  private static Decimal rate = 1.2;

  public static Decimal withTax(Decimal net) {
    // A static used without its class name, too short for the textual tier.
    return MathUtil.round(net * rate, 2);
  }

  public static Boolean canSeeTotals() {
    return Schema.SObjectType.Invoice__c.fields.Total__c.isAccessible() &&
      Schema.Invoice__c.Paid__c.getDescribe().isAccessible();
  }

  public static String title() {
    return System.Label.Tariff_Title;
  }
}
