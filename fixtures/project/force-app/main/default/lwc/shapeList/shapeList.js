import { LightningElement, wire } from 'lwc';
import describeAll from '@salesforce/apex/ShapeService.describeAll';

export default class ShapeList extends LightningElement {
  @wire(describeAll, { shapes: [] }) lines;
}
