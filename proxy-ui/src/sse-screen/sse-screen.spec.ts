import { ComponentFixture, TestBed } from '@angular/core/testing';

import { SseScreen } from './sse-screen';

describe('SseScreen', () => {
  let component: SseScreen;
  let fixture: ComponentFixture<SseScreen>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [SseScreen]
    })
    .compileComponents();

    fixture = TestBed.createComponent(SseScreen);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
