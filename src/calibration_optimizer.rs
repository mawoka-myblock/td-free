use log;

// Simple optimization structure for slider calibration
#[derive(Debug, Clone, Copy)]
pub struct SliderParams {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub brightness: f32,
}

impl SliderParams {
    pub fn new(red: f32, green: f32, blue: f32, brightness: f32) -> Self {
        Self { red, green, blue, brightness }
    }

    pub fn clamp(&mut self) {
        self.red = self.red.max(0.1).min(5.0);
        self.green = self.green.max(0.1).min(5.0);
        self.blue = self.blue.max(0.1).min(5.0);
        self.brightness = self.brightness.max(0.1).min(5.0);
    }
}

// Calculate color distance loss function (without logging for internal use)
fn calculate_color_loss_internal(
    current_rgb: (u8, u8, u8),
    target_rgb: (u8, u8, u8),
    params: &SliderParams,
    current_td: f32,
    td_reference: f32
) -> f32 {
    // Apply the slider parameters to get predicted output color
    let predicted_rgb = apply_slider_params_to_color(current_rgb, params, current_td, td_reference);
    
    // Calculate Euclidean distance in RGB space
    let r_diff = predicted_rgb.0 as f32 - target_rgb.0 as f32;
    let g_diff = predicted_rgb.1 as f32 - target_rgb.1 as f32;
    let b_diff = predicted_rgb.2 as f32 - target_rgb.2 as f32;
    
    (r_diff * r_diff + g_diff * g_diff + b_diff * b_diff).sqrt()
}

// Calculate color distance loss function
pub fn calculate_color_loss(
    current_rgb: (u8, u8, u8),
    target_rgb: (u8, u8, u8),
    params: &SliderParams,
    current_td: f32,
    td_reference: f32
) -> f32 {
    let loss = calculate_color_loss_internal(current_rgb, target_rgb, params, current_td, td_reference);
    let predicted_rgb = apply_slider_params_to_color(current_rgb, params, current_td, td_reference);
    
    log::info!("Color loss: Current({},{},{}) -> Predicted({},{},{}) vs Target({},{},{}) = {:.2}",
               current_rgb.0, current_rgb.1, current_rgb.2,
               predicted_rgb.0, predicted_rgb.1, predicted_rgb.2,
               target_rgb.0, target_rgb.1, target_rgb.2, loss);
    
    loss
}

// Apply slider parameters to a color (simulates the frontend color correction)
pub fn apply_slider_params_to_color(
    rgb: (u8, u8, u8),
    params: &SliderParams,
    current_td: f32,
    td_reference: f32
) -> (u8, u8, u8) {
    // Simulate the TD-based brightness correction
    let td_brightness_factor = if td_reference > 0.1 {
        td_reference / current_td.max(0.1)
    } else {
        1.0
    };
    
    // Apply color multipliers and brightness
    let total_brightness = params.brightness * td_brightness_factor;
    
    let r_final = (rgb.0 as f32 * params.red * total_brightness).round().min(255.0).max(0.0) as u8;
    let g_final = (rgb.1 as f32 * params.green * total_brightness).round().min(255.0).max(0.0) as u8;
    let b_final = (rgb.2 as f32 * params.blue * total_brightness).round().min(255.0).max(0.0) as u8;
    
    (r_final, g_final, b_final)
}

// Adam optimizer state
#[derive(Debug, Clone, Copy)]
struct AdamState {
    m: [f32; 4], // First moment (momentum)
    v: [f32; 4], // Second moment (velocity)
    t: usize,    // Time step
}

impl AdamState {
    fn new() -> Self {
        Self {
            m: [0.0; 4],
            v: [0.0; 4],
            t: 0,
        }
    }
}

// Compute analytical gradients of the loss function
fn compute_analytical_gradients(
    current_rgb: (u8, u8, u8),
    target_rgb: (u8, u8, u8),
    params: &SliderParams,
    current_td: f32,
    td_reference: f32,
) -> [f32; 4] {
    // Calculate the TD-based brightness factor
    let td_brightness_factor = if td_reference > 0.1 {
        td_reference / current_td.max(0.1)
    } else {
        1.0
    };
    
    let total_brightness = params.brightness * td_brightness_factor;
    
    // Current predicted RGB values (before clamping for gradient computation)
    let r_pred_raw = current_rgb.0 as f32 * params.red * total_brightness;
    let g_pred_raw = current_rgb.1 as f32 * params.green * total_brightness;
    let b_pred_raw = current_rgb.2 as f32 * params.blue * total_brightness;
    
    // Apply clamping to get actual predicted values
    let r_pred = r_pred_raw.min(255.0).max(0.0);
    let g_pred = g_pred_raw.min(255.0).max(0.0);
    let b_pred = b_pred_raw.min(255.0).max(0.0);
    
    // Calculate error terms
    let r_error = r_pred - target_rgb.0 as f32;
    let g_error = g_pred - target_rgb.1 as f32;
    let b_error = b_pred - target_rgb.2 as f32;
    
    // Current loss value for normalization
    let current_loss = (r_error * r_error + g_error * g_error + b_error * b_error).sqrt();
    
    // Avoid division by zero
    if current_loss < 1e-8 {
        return [0.0; 4];
    }
    
    // Derivative of sqrt(sum of squares) = (1 / (2 * sqrt(sum))) * d(sum)/dx
    // For each parameter, we need d(loss)/d(param)
    
    let mut gradients = [0.0; 4];
    
    // Gradient w.r.t. red multiplier
    // d(loss)/d(red) = (1/loss) * r_error * d(r_pred)/d(red)
    // d(r_pred)/d(red) = current_rgb.0 * total_brightness (if not clamped)
    if r_pred_raw >= 0.0 && r_pred_raw <= 255.0 {
        gradients[0] = (r_error / current_loss) * (current_rgb.0 as f32 * total_brightness);
    }
    
    // Gradient w.r.t. green multiplier
    if g_pred_raw >= 0.0 && g_pred_raw <= 255.0 {
        gradients[1] = (g_error / current_loss) * (current_rgb.1 as f32 * total_brightness);
    }
    
    // Gradient w.r.t. blue multiplier
    if b_pred_raw >= 0.0 && b_pred_raw <= 255.0 {
        gradients[2] = (b_error / current_loss) * (current_rgb.2 as f32 * total_brightness);
    }
    
    // Gradient w.r.t. brightness
    // d(loss)/d(brightness) = (1/loss) * sum(error_i * d(pred_i)/d(brightness))
    // d(pred_i)/d(brightness) = current_rgb.i * params.i * td_brightness_factor
    let mut brightness_grad = 0.0;
    if r_pred_raw >= 0.0 && r_pred_raw <= 255.0 {
        brightness_grad += r_error * (current_rgb.0 as f32 * params.red * td_brightness_factor);
    }
    if g_pred_raw >= 0.0 && g_pred_raw <= 255.0 {
        brightness_grad += g_error * (current_rgb.1 as f32 * params.green * td_brightness_factor);
    }
    if b_pred_raw >= 0.0 && r_pred_raw <= 255.0 {
        brightness_grad += b_error * (current_rgb.2 as f32 * params.blue * td_brightness_factor);
    }
    gradients[3] = brightness_grad / current_loss;
    
    gradients
}

// Adam optimizer update step
fn adam_update(
    params: &mut SliderParams,
    gradients: &[f32; 4],
    state: &mut AdamState,
    learning_rate: f32,
    beta1: f32,
    beta2: f32,
    epsilon: f32,
) {
    state.t += 1;
    
    for i in 0..4 {
        // Update biased first moment estimate
        state.m[i] = beta1 * state.m[i] + (1.0 - beta1) * gradients[i];
        
        // Update biased second moment estimate
        state.v[i] = beta2 * state.v[i] + (1.0 - beta2) * gradients[i] * gradients[i];
        
        // Compute bias-corrected first moment estimate
        let m_hat = state.m[i] / (1.0 - beta1.powi(state.t as i32));
        
        // Compute bias-corrected second moment estimate
        let v_hat = state.v[i] / (1.0 - beta2.powi(state.t as i32));
        
        // Update parameters
        let update = learning_rate * m_hat / (v_hat.sqrt() + epsilon);
        
        match i {
            0 => params.red -= update,
            1 => params.green -= update,
            2 => params.blue -= update,
            3 => params.brightness -= update,
            _ => unreachable!(),
        }
    }
    
    params.clamp();
}

// Simple gradient-free optimizer using coordinate descent with random perturbations
pub fn optimize_sliders(
    current_rgb: (u8, u8, u8),
    target_rgb: (u8, u8, u8),
    initial_params: SliderParams,
    current_td: f32,
    td_reference: f32,
    max_iterations: usize
) -> SliderParams {
    let mut params = initial_params;
    let mut adam_state = AdamState::new();
    let mut best_loss = calculate_color_loss(current_rgb, target_rgb, &params, current_td, td_reference);
    
    log::info!("Starting Adam optimization with analytical gradients: Initial loss = {:.2}", best_loss);
    log::info!("Initial conditions: current_td={:.3}, td_reference={:.3}", current_td, td_reference);
    
    // Adam hyperparameters
    let learning_rate = 0.1;
    let beta1 = 0.9;
    let beta2 = 0.999;
    let epsilon = 1e-8;
    
    for iteration in 0..max_iterations {
        // Compute analytical gradients
        let gradients = compute_analytical_gradients(
            current_rgb,
            target_rgb,
            &params,
            current_td,
            td_reference,
        );
        
        // Log gradients for debugging
        log::info!("Iteration {}: Gradients = [{:.6}, {:.6}, {:.6}, {:.6}]",
                  iteration, gradients[0], gradients[1], gradients[2], gradients[3]);
        
        // Check for convergence based on gradient magnitude
        let gradient_magnitude: f32 = gradients.iter().map(|g| g * g).sum::<f32>().sqrt();
        
        if gradient_magnitude < 1e-6 {
            log::info!("Adam optimization converged at iteration {} (gradient magnitude < 1e-6)", iteration);
            break;
        }
        
        // Apply Adam update
        adam_update(
            &mut params,
            &gradients,
            &mut adam_state,
            learning_rate,
            beta1,
            beta2,
            epsilon,
        );
        
        let current_loss = calculate_color_loss_internal(current_rgb, target_rgb, &params, current_td, td_reference);
        
        if current_loss < best_loss {
            best_loss = current_loss;
        }
        
        // Early termination if loss is very small
        if best_loss < 1.0 {
            log::info!("Adam optimization converged at iteration {} (loss < 1.0)", iteration);
            break;
        }
        
        // Log progress every iteration for better monitoring
        log::info!("Iteration {}: Loss = {:.2}, Gradient magnitude = {:.6}, Params = ({:.3},{:.3},{:.3},{:.3})",
                  iteration, current_loss, gradient_magnitude, params.red, params.green, params.blue, params.brightness);
        
        // Additional convergence check: if loss hasn't improved significantly in recent iterations
        if iteration > 10 && gradient_magnitude < 0.01 {
            log::info!("Adam optimization converged at iteration {} (small gradients and sufficient iterations)", iteration);
            break;
        }
    }
    
    // Calculate final loss with logging for verification
    let final_loss = calculate_color_loss(current_rgb, target_rgb, &params, current_td, td_reference);
    
    log::info!("Adam optimization completed: Final loss = {:.2}, Final params = ({:.3},{:.3},{:.3},{:.3})",
              final_loss, params.red, params.green, params.blue, params.brightness);
    
    params
}
