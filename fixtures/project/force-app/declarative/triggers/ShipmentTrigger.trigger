trigger ShipmentTrigger on Shipment__c (before insert) {
  new ShipmentAudit().run();
}
