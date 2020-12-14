use crate::error::Error;
use crate::math_helper;
use crate::structs;
use ndarray::{Array1, Array2, ArrayBase, Dim, ViewRepr};
use std::str;
use unicode_segmentation::UnicodeSegmentation;

pub struct Lstm {
    data: structs::LstmData,
}

#[derive(Debug)]
pub struct TestText {
    pub data: structs::TestTextData,
}

impl TestText {
    pub fn new(data: structs::TestTextData) -> Self {
        Self { data }
    }
}

impl Lstm {
    /// `try_new` is the initiator of struct `Lstm`
    pub fn try_new(data: structs::LstmData) -> Result<Self, Error> {
        if data.dic.len() > std::i16::MAX as usize {
            return Err(Error::Limit);
        }
        Ok(Self { data })
    }

    /// `get_model_name` returns the name of the LSTM model.
    pub fn get_model_name(&self) -> &str {
        &self.data.model
    }

    /// `compute_bies` uses the computed probabilities of BIES and pick the letter with the largest probability
    fn compute_bies(&self, arr: Array1<f32>) -> Result<char, Error> {
        let ind = math_helper::max_arr1(arr.view());
        match ind {
            0 => Ok('b'),
            1 => Ok('i'),
            2 => Ok('e'),
            3 => Ok('s'),
            _ => Err(Error::Syntax),
        }
    }

    /// `_return_id` returns the id corresponding to a code point or a grapheme cluster based on the model dictionary.
    fn return_id(&self, g: &str) -> i16 {
        let g_id: i16 = if self.data.dic.contains_key(g) {
            self.data.dic[g]
        } else {
            self.data.dic.len() as i16
        };
        g_id
    }

    /// `compute_hc1` implemens the evaluation of one LSTM layer.
    fn compute_hc(
        &self,
        x_t: ArrayBase<ViewRepr<&f32>, Dim<[usize; 1]>>,
        h_tm1: &Array1<f32>,
        c_tm1: &Array1<f32>,
        warr: ArrayBase<ViewRepr<&f32>, Dim<[usize; 2]>>,
        uarr: ArrayBase<ViewRepr<&f32>, Dim<[usize; 2]>>,
        barr: ArrayBase<ViewRepr<&f32>, Dim<[usize; 1]>>,
    ) -> (Array1<f32>, Array1<f32>) {
        // i, f, and o respectively stand for input, forget, and output gates
        let s_t = x_t.dot(&warr) + h_tm1.dot(&uarr) + barr;
        let hunits = uarr.shape()[0];
        let i = math_helper::sigmoid_arr1(s_t.slice(ndarray::s![..hunits]));
        let f = math_helper::sigmoid_arr1(s_t.slice(ndarray::s![hunits..2 * hunits]));
        let _c = math_helper::tanh_arr1(s_t.slice(ndarray::s![2 * hunits..3 * hunits]));
        let o = math_helper::sigmoid_arr1(s_t.slice(ndarray::s![3 * hunits..]));
        let c_t = i * _c + f * c_tm1;
        let h_t = o * math_helper::tanh_arr1(c_t.view());
        (h_t, c_t)
    }

    /// `word_segmenter` is a function that gets a "clean" unsegmented string as its input and returns a BIES (B: Beginning, I: Inside, E: End,
    /// S: Single) sequence for grapheme clusters. The boundaries of words can be found easily using this BIES sequence.
    pub fn word_segmenter(&self, input: &str) -> String {
        // input_seq is a sequence of id numbers that represents grapheme clusters or code points in the input line. These ids are used later
        // in the embedding layer of the model.
        let input_seq: Result<Vec<i16>, Error> = if self.data.model.contains("codepoints") {
            Ok(input
                .chars()
                .map(|c| self.return_id(&c.to_string()))
                .collect())
        } else if self.data.model.contains("graphclust") {
            Ok(UnicodeSegmentation::graphemes(input, true)
                .map(|s| self.return_id(s))
                .collect())
        } else {
            Err(Error::Syntax)
        };

        // x_data is the data ready to be feed into the model
        let x_data = input_seq.unwrap();
        let x_data_len = x_data.len();

        // hunits is the number of hidden unints in each LSTM cell
        let hunits = self.data.mat3.shape()[0];
        // Forward LSTM
        let mut c_fw = Array1::<f32>::zeros(hunits);
        let mut h_fw = Array1::<f32>::zeros(hunits);
        let mut all_h_fw = Array2::<f32>::zeros((x_data_len, hunits));
        for (i, g_id) in x_data.iter().enumerate() {
            let x_t = self.data.mat1.slice(ndarray::s![*g_id as isize, ..]);
            let (new_h, new_c) = self.compute_hc(
                x_t,
                &h_fw,
                &c_fw,
                self.data.mat2.view(),
                self.data.mat3.view(),
                self.data.mat4.view(),
            );
            h_fw = new_h;
            c_fw = new_c;
            all_h_fw = math_helper::change_row(all_h_fw, i, &h_fw);
        }

        // Backward LSTM
        let mut c_bw = Array1::<f32>::zeros(hunits);
        let mut h_bw = Array1::<f32>::zeros(hunits);
        let mut all_h_bw = Array2::<f32>::zeros((x_data_len, hunits));
        for (i, g_id) in x_data.iter().rev().enumerate() {
            let x_t = self.data.mat1.slice(ndarray::s![*g_id as isize, ..]);
            let (new_h, new_c) = self.compute_hc(
                x_t,
                &h_bw,
                &c_bw,
                self.data.mat5.view(),
                self.data.mat6.view(),
                self.data.mat7.view(),
            );
            h_bw = new_h;
            c_bw = new_c;
            all_h_bw = math_helper::change_row(all_h_bw, x_data_len - 1 - i, &h_bw);
        }

        // Combining forward and backward LSTMs using the dense time-distributed layer
        let timew = self.data.mat8.view();
        let timeb = self.data.mat9.view();
        let mut bies = String::from("");
        for i in 0..x_data_len {
            let curr_fw = all_h_fw.slice(ndarray::s![i, ..]);
            let curr_bw = all_h_bw.slice(ndarray::s![i, ..]);
            let concat_lstm = math_helper::concatenate_arr1(curr_fw, curr_bw);
            let curr_est = concat_lstm.dot(&timew) + timeb;
            let probs = math_helper::softmax(curr_est);
            bies.push(self.compute_bies(probs).unwrap());
        }
        bies
    }
}