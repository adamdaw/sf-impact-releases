// Bound by the metadata record Shipping_Plugin.Tracking: no Apex calls it.
public with sharing class ShipmentTracker {
  public List<Shipment__c> late(Id carrierId) {
    return [
      SELECT Id, Weight__c, Carrier__r.Region__c, Payer__r.Region__c, Owner.Alias,
        (SELECT Weight__c FROM Parcels__r)
      FROM Shipment__c
      WHERE Carrier__c = :carrierId AND Delivered__c = FALSE
    ];
  }

  public void deliver(List<Shipment__c> shipments) {
    upsert shipments Tracking_Id__c;
    insert new Parcel__c(Weight__c = 1, Colour__c = 'red');
  }

  public List<Shipment__c> payers() {
    return [SELECT Payer__r.No__c, Payer__r.Name FROM Shipment__c WHERE Kg__c > 100];
  }

  public Boolean isConfigured() {
    Shipping_Settings__c settings = Shipping_Settings__c.getOrgDefaults();
    return settings.get('Default_Region__c') != null && !Shipping_Plugin__mdt.getAll().isEmpty();
  }

  public String title() {
    Schema.SObjectField weight = Schema.Shipment__c.Weight__c;
    Schema.SObjectField delivered = Shipment__c.Delivered__c;
    return Label.Shipment_Title + weight + delivered;
  }
}
