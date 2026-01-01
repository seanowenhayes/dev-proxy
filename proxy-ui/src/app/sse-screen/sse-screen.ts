import { Component, inject } from '@angular/core';
import { SseService } from './sse-service';

@Component({
  selector: 'app-sse-screen',
  imports: [],
  templateUrl: './sse-screen.html',
  styleUrl: './sse-screen.css',
})
export class SseScreen {
  sseService = inject(SseService);

  carrot() {
    this.sseService.connectSSE().subscribe((data) => console.log(data));
  }
}
